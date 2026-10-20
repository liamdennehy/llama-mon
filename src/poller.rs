//! Polling engine — orchestrates fetches, deduplication, reload
//! detection, and persistence.
//!
//! The poller runs a loop that:
//! 1. Fetches `/metrics` to discover active models and their counters
//! 2. Detects reloads via `tasks.completed` counter decrease → re-fetch `/models`
//! 3. Upserts model metadata (from `/models` on first/load, from metrics otherwise)
//! 4. Deduplicates new tasks via `task_ids_seen` CSV
//! 5. Persists tasks and scrapes to SQLite
//! 6. Marks DB-only models (not in metrics.active) as unloaded (lifecycle)

use thiserror::Error;
use tokio::sync::broadcast;
use tokio::sync::mpsc::UnboundedReceiver;

use crate::config::{Config, ServerConfig};
use crate::db;
use crate::db::model_load::{self, ModelLoadInsert};
use crate::db::task::TaskInsert;
use crate::http_client::HttpClient;
use crate::metrics_api;
use crate::parse::{ModelEntry, ModelLoad};
use crate::models_api;

/// Signal actions dispatched by the signal handler in `main.rs`.
pub enum SignalAction {
    /// SIGINT / SIGTERM — initiate graceful shutdown.
    Shutdown,
    /// SIGHUP — re-read and log configuration.
    DumpConfig,
    /// SIGUSR1 — dump current poll stats to log.
    DumpStats,
}

#[derive(Error, Debug)]
pub enum PollerError {
    #[error("HTTP error: {0}")]
    Http(#[from] crate::http_client::HttpError),
    #[error("Database error: {0}")]
    Db(#[from] crate::db::DbError),
    #[error("Poller stopped")]
    Stopped,
}

/// Cumulative statistics tracked by the poller.
#[derive(Debug, Default, Clone)]
pub struct PollStats {
    pub total_scrapes: u64,
    pub total_tasks: u64,
    pub total_model_loads: u64,
    pub total_poll_cycles: u64,
    pub total_reloads: u64,
}

/// Event returned by reload detection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReloadEvent {
    /// The `tasks.completed` counter increased or stayed the same —
    /// still the same model load session.
    SameSession {
        /// The current `tasks.completed` value.
        completed: i64,
    },
    /// The `tasks.completed` counter decreased — a model reload
    /// or new session started.
    ReloadDetected {
        /// Previous completed counter.
        previous_completed: i64,
        /// New completed counter (lower value).
        new_completed: i64,
    },
    /// First scrape for this model — no previous counter to compare.
    FirstScrape,
}

/// The polling engine. Holds the HTTP client, database pool,
/// and cumulative stats.
pub struct Poller {
    client: HttpClient,
    server: ServerConfig,
    config: Config,
    pool: sqlx::SqlitePool,
    stats: PollStats,
}

impl Poller {
    /// Create a new poller for a specific server.
    pub fn new(server: ServerConfig, config: Config, pool: sqlx::SqlitePool) -> Self {
        let client = HttpClient::new(
            server.url.clone(),
            config.timeout_duration(),
        );
        Self {
            client,
            server,
            config,
            pool,
            stats: PollStats::default(),
        }
    }

    /// Return the server identifier for logging.
    pub fn server_id(&self) -> String {
        self.server.identifier()
    }

    /// Return a reference to the database pool (for testing).
    pub fn pool(&self) -> &sqlx::SqlitePool {
        &self.pool
    }

    /// Run the main polling loop. Returns on shutdown.
    ///
    /// Handles three signal types:
    /// - Shutdown (SIGINT/SIGTERM) — stop the loop
    /// - DumpConfig (SIGHUP) — re-log the current configuration
    /// - DumpStats (SIGUSR1) — log cumulative poll stats
    pub async fn run(&mut self, mut action_rx: UnboundedReceiver<SignalAction>) -> Result<(), PollerError> {
        tracing::info!(
            poll_interval = ?self.config.poll_duration(),
            "Poller started"
        );

        loop {
            let result = tokio::select! {
                _ = action_rx.recv() => {
                    // Handle pending signals before polling
                    loop {
                        match action_rx.try_recv() {
                            Ok(action) => match action {
                                SignalAction::Shutdown => {
                                    tracing::info!("Shutdown signal received");
                                    // Graceful shutdown
                                    tracing::info!(
                                        total_scrapes = self.stats.total_scrapes,
                                        total_tasks = self.stats.total_tasks,
                                        total_model_loads = self.stats.total_model_loads,
                                        total_reloads = self.stats.total_reloads,
                                        "Graceful shutdown"
                                    );
                                    if let Err(e) = db::wal_checkpoint(&self.pool).await {
                                        tracing::warn!(error = %e, "Failed to checkpoint WAL on shutdown");
                                    }
                                    return Err(PollerError::Stopped);
                                }
                                SignalAction::DumpConfig => {
                                    tracing::info!(
                                        server_url = %self.client.base_url(),
                                        poll_interval = ?self.config.poll_duration(),
                                        timeout = ?self.config.timeout_duration(),
                                        verbose = self.config.verbose,
                                        model_filter = ?self.config.models,
                                        fail_fast = self.config.fail_fast,
                                        "Configuration (SIGHUP)"
                                    );
                                }
                                SignalAction::DumpStats => {
                                    tracing::info!(
                                        total_scrapes = self.stats.total_scrapes,
                                        total_tasks = self.stats.total_tasks,
                                        total_model_loads = self.stats.total_model_loads,
                                        total_poll_cycles = self.stats.total_poll_cycles,
                                        total_reloads = self.stats.total_reloads,
                                        "Poll stats (SIGUSR1)"
                                    );
                                }
                            },
                            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => break,
                            Err(e) => {
                                tracing::warn!(error = %e, "Error reading signal action");
                                break;
                            }
                        }
                    }
                    continue;
                }
                result = self.poll_cycle() => result,
            };

            match result {
                Ok(stats) => {
                    self.stats = stats;
                }
                Err(e) => {
                    tracing::error!(error = %e, "Poll cycle failed");
                    // If --fail-fast is set, exit immediately instead of retrying
                    if self.config.fail_fast {
                        tracing::warn!("Fail-fast mode — exiting after error");
                        if let Err(wal_err) = db::wal_checkpoint(&self.pool).await {
                            tracing::warn!(error = %wal_err, "Failed to checkpoint WAL on exit");
                        }
                        return Err(PollerError::Stopped);
                    }
                }
            }

            tokio::time::sleep(self.config.poll_duration()).await;
        }

        // The loop only exits via `return` on shutdown signal.
        // This line is technically unreachable but required for type inference.
        #[allow(unreachable_code)]
        Err(PollerError::Stopped)
    }

    /// Run the poller with a broadcast-based shutdown channel.
    ///
    /// Like `run()` but uses a broadcast channel receiver instead of
    /// an unbounded mpsc receiver for the shutdown signal. This allows
    /// sharing a single shutdown channel between the poller and the
    /// web server.
    pub async fn run_with_shutdown(
        &mut self,
        mut shutdown_rx: broadcast::Receiver<()>,
    ) -> Result<(), PollerError> {
        let poll_duration = self.config.poll_duration();
        loop {
            // Check for shutdown before polling
            if shutdown_rx.try_recv().is_ok() {
                tracing::info!("Shutdown signal received");
                tracing::info!(
                    total_scrapes = self.stats.total_scrapes,
                    total_tasks = self.stats.total_tasks,
                    total_model_loads = self.stats.total_model_loads,
                    total_reloads = self.stats.total_reloads,
                    "Graceful shutdown"
                );
                if let Err(e) = db::wal_checkpoint(&self.pool).await {
                    tracing::warn!(error = %e, "Failed to checkpoint WAL on shutdown");
                }
                return Err(PollerError::Stopped);
            }

            match self.poll_cycle().await {
                Ok(_) => {} // Cycle complete
                Err(e) => tracing::error!(error = %e, "Poll cycle failed"),
            }

            // Sleep for the poll interval, but check shutdown during the wait
            let sleep_fut = tokio::time::sleep(poll_duration);
            tokio::select! {
                _ = shutdown_rx.recv() => {
                    tracing::info!("Shutdown signal received during sleep");
                    if let Err(e) = db::wal_checkpoint(&self.pool).await {
                        tracing::warn!(error = %e, "Failed to checkpoint WAL on shutdown");
                    }
                    return Err(PollerError::Stopped);
                }
                _ = sleep_fut => {
                    // Sleep expired — loop back to the next poll cycle
                }
            }
        }
    }

    /// Run the poller for exactly `max_cycles` cycles, then return stats.
    ///
    /// This is a test-friendly variant of `run()` that does not require
    /// signal handling. It runs a fixed number of cycles and then exits
    /// cleanly with `PollStats`.
    /// Run the poller for exactly `max_cycles` cycles, then return stats.
    ///
    /// This is a test-friendly variant of `run()` that does not require
    /// signal handling. It runs a fixed number of cycles and then exits
    /// cleanly with `PollStats`.
    pub async fn run_until(
        &mut self,
        max_cycles: u64,
    ) -> Result<PollStats, PollerError> {
        tracing::info!(max_cycles, "Poller started (test mode)");
        let mut cycle = 0u64;
        while cycle < max_cycles {
            cycle += 1;
            let stats = self.poll_cycle().await?;
            self.stats = stats;
            // Don't sleep after the last cycle
            if cycle < max_cycles {
                tokio::time::sleep(self.config.poll_duration()).await;
            }
        }
        tracing::info!(
            total_cycles = cycle,
            total_scrapes = self.stats.total_scrapes,
            total_tasks = self.stats.total_tasks,
            "Poller finished (test mode)"
        );
        Ok(self.stats.clone())
    }

    /// Execute a single poll cycle.
    ///
    /// 1. Fetch `/metrics` to discover active models and their counters
    /// 2. For each active model, detect reloads via `tasks.completed` decrease
    /// 3. On reload: fetch `/models` for fresh metadata
    /// 4. Upsert model loads and persist tasks/scrapes
    /// 5. Mark DB-only models (not in metrics.active) as unloaded
    async fn poll_cycle(&mut self) -> Result<PollStats, PollerError> {
        let cycle_span = tracing::info_span!("poll_cycle");
        let _enter = cycle_span.enter();
        let cycle_start = std::time::Instant::now();

        let mut new_loads = 0u64;
        let mut total_new_tasks = 0u64;
        let mut total_scrapes = 0u64;
        let unloaded;

        // Step 1: Fetch aggregated metrics
        let metrics_resp = match metrics_api::fetch_metrics(&self.client).await {
            Ok(resp) => resp,
            Err(e) => {
                match &e {
                    crate::http_client::HttpError::Status(code, _) if *code >= 400 && *code < 500 => {
                        tracing::error!(status = code, error = %e, "/metrics returned 4xx — skipping this cycle");
                        return Ok(self.stats.clone());
                    }
                    crate::http_client::HttpError::Status(code, _) if *code >= 500 => {
                        tracing::error!(status = code, error = %e, "/metrics returned 5xx — will retry next cycle");
                        return Ok(self.stats.clone());
                    }
                    _ => {
                        tracing::error!(error = %e, "Failed to fetch /metrics");
                        return Err(PollerError::Http(e));
                    }
                }
            }
        };

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock went backwards")
            .as_secs() as i64;

        // Collect active model names from metrics
        let active_model_names: Vec<String> = metrics_resp.active.iter()
            .map(|m| m.model.clone())
            .collect();

        // Step 2: Process each active model from metrics
        for active in &metrics_resp.active {
            // Model filter
            if !self.config.model_matches(&active.model) {
                tracing::debug!(model = active.model, "Skipped by model filter");
                continue;
            }

            let full_metrics = &active.metrics;
            let model_name = &active.model;

            // Reload detection
            let reload_event = self.detect_reload(model_name, full_metrics.tasks.completed).await?;

            match &reload_event {
                ReloadEvent::FirstScrape => {
                    tracing::info!(
                        model = model_name,
                        completed = full_metrics.tasks.completed,
                        "First scrape — no prior scrape found"
                    );
                }
                ReloadEvent::ReloadDetected { previous_completed, new_completed } => {
                    tracing::info!(
                        model = model_name,
                        previous_completed,
                        new_completed,
                        "Reload detected — fetching /models for new metadata"
                    );

                    // Fetch /models to get new load metadata
                    let models_resp = match models_api::fetch_models(&self.client).await {
                        Ok(resp) => Some(resp),
                        Err(e) => {
                            tracing::warn!(error = %e, "Failed to fetch /models after reload — using metrics-only metadata");
                            None
                        }
                    };

                    // Close old session
                    let old_id = self.get_current_load_id(model_name).await?;
                    if let Some(old_id) = old_id {
                        model_load::close(&self.pool, old_id, now).await?;
                    }

                    // Find metadata from /models if available
                    let (load, entry) = models_resp
                        .as_ref()
                        .and_then(|mr| mr.data.iter().find(|e| &e.model == model_name))
                        .map(|e| {
                            let load = ModelLoad::from_entry(e);
                            (load, e.clone())
                        })
                        .unwrap_or_else(|| {
                            // Fallback: minimal metadata from metrics alone
                            let load = crate::parse::ModelLoad {
                                model_name: model_name.clone(),
                                created: 0,
                                status: crate::parse::ModelLoadStatus::Loaded,
                                preset: Default::default(),
                            };
                            let entry = crate::parse::ModelEntry {
                                model: model_name.clone(),
                                created: 0,
                                status: crate::parse::ModelStatus {
                                    value: "loaded".to_string(),
                                    args: None,
                                    preset: None,
                                },
                                preset: None,
                            };
                            (load, entry)
                        });

                    let new_created = entry.created + 1;
                    let (new_load_id, was_new) = model_load::upsert(&self.pool, &model_load_to_insert(&load, &entry)).await?;
                    if was_new {
                        new_loads += 1;
                    }

                    let mut scrape_insert: db::scrape::ScrapeInsert =
                        (full_metrics, now).into();
                    scrape_insert.model_name = model_name.clone();
                    scrape_insert.model_created = new_created;
                    scrape_insert.model_load_id = new_load_id;

                    let (new_tasks, scrape_id) = self.process_metrics(
                        model_name, new_created, new_load_id,
                        &full_metrics.recent, &scrape_insert,
                    ).await?;
                    total_new_tasks += new_tasks;
                    total_scrapes += if scrape_id > 0 { 1 } else { 0 };
                }
                ReloadEvent::SameSession { completed } => {
                    tracing::debug!(
                        model = model_name,
                        completed,
                        "Same session — no /models fetch needed"
                    );

                    // Normal same-session processing — use DB metadata
                    let load_id = self.get_current_load_id(model_name).await?;

                    if let Some(load_id) = load_id {
                        let mut scrape_insert: db::scrape::ScrapeInsert =
                            (full_metrics, now).into();
                        scrape_insert.model_name = model_name.clone();
                        scrape_insert.model_created = 0; // set from DB
                        scrape_insert.model_load_id = load_id;

                        // Update model_created from DB
                        if let Ok(Some(model_created)) = sqlx::query_scalar::<_, i64>(
                            "SELECT model_created FROM model_loads WHERE id = ?"
                        )
                        .bind(load_id)
                        .fetch_optional(&self.pool)
                        .await
                        {
                            scrape_insert.model_created = model_created;
                        }

                        let (new_tasks, scrape_id) = self.process_metrics(
                            model_name, scrape_insert.model_created, load_id,
                            &full_metrics.recent, &scrape_insert,
                        ).await?;
                        total_new_tasks += new_tasks;
                        total_scrapes += if scrape_id > 0 { 1 } else { 0 };
                    }
                }
            }
        }

        // Step 3: Lifecycle transitions — unload models in DB but not in metrics.active
        unloaded = self.check_lifecycle_transitions(&active_model_names).await?;

        // Update stats
        self.stats.total_scrapes += total_scrapes;
        self.stats.total_tasks += total_new_tasks;
        self.stats.total_model_loads += new_loads;
        self.stats.total_poll_cycles += 1;

        let elapsed = cycle_start.elapsed();

        tracing::info!(
            elapsed_ms = elapsed.as_millis(),
            new_tasks = total_new_tasks,
            scrapes = total_scrapes,
            new_loads = new_loads,
            unloaded = unloaded,
            total_model_loads = self.stats.total_model_loads,
            "Poll cycle completed"
        );

        Ok(self.stats.clone())
    }

    /// Get the current active model_load id for a model name.
    /// Returns the most recent non-unloaded load, or None if none exists.
    async fn get_current_load_id(&self, model_name: &str) -> Result<Option<i64>, PollerError> {
        let id: Option<i64> = sqlx::query_scalar::<_, i64>(
            r#"SELECT id FROM model_loads
               WHERE model_name = ? AND status != 'unloaded'
               ORDER BY created DESC LIMIT 1"#
        )
        .bind(model_name)
        .fetch_optional(&self.pool)
        .await
        .map_err(db::DbError::from)?;
        Ok(id)
    }

    /// Check for lifecycle transitions: models in DB but not in metrics.active
    /// should be marked as unloaded.
    async fn check_lifecycle_transitions(&mut self, active_model_names: &[String]) -> Result<u64, PollerError> {
        let mut unloaded = 0u64;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock went backwards")
            .as_secs() as i64;

        // Get all model loads that are currently loaded or sleeping
        let rows: Vec<db::model_load::ModelLoadRow> = sqlx::query_as(
            "SELECT * FROM model_loads WHERE status IN ('loaded', 'sleeping')"
        )
        .fetch_all(&self.pool)
        .await.map_err(db::DbError::from)?;

        for row in rows {
            if !active_model_names.contains(&row.model_name) {
                model_load::close(&self.pool, row.id, now).await?;
                unloaded += 1;
                tracing::info!(
                    model = row.model_name,
                    load_id = row.id,
                    "Model no longer in /metrics active — marked unloaded"
                );
            }
        }

        Ok(unloaded)
    }

    /// Detect whether a model reload occurred by comparing
    /// the `tasks.completed` counter against the last known value.
    async fn detect_reload(
        &self,
        model_name: &str,
        current_completed: i64,
    ) -> Result<ReloadEvent, PollerError> {
        let last_completed = db::task::get_last_completed_counter(
            &self.pool, model_name,
        ).await?;

        match last_completed {
            None => Ok(ReloadEvent::FirstScrape),
            Some(previous) => {
                if current_completed < previous {
                    Ok(ReloadEvent::ReloadDetected {
                        previous_completed: previous,
                        new_completed: current_completed,
                    })
                } else {
                    Ok(ReloadEvent::SameSession {
                        completed: current_completed,
                    })
                }
            }
        }
    }

    /// Process metrics: deduplicate tasks, persist, record scrape.
    ///
    /// Even if there are no new tasks, a scrape is still recorded when
    /// `recent_tasks` is non-empty. If `recent_tasks` is empty, we still
    /// record the scrape with zero aggregates for completeness.
    ///
    /// Returns (new_tasks_inserted, scrape_id).
    async fn process_metrics(
        &self,
        model_name: &str,
        _model_created: i64,
        model_load_id: i64,
        recent_tasks: &[crate::parse::TaskRecord],
        scrape_insert: &db::scrape::ScrapeInsert,
    ) -> Result<(u64, i64), PollerError> {
        let has_recent = !recent_tasks.is_empty();

        if !has_recent {
            // Edge case: no recent tasks — still record a scrape for completeness
            // but skip dedup and insert steps
            let scrape_id = db::scrape::insert(&self.pool, scrape_insert).await?;
            tracing::debug!(model = model_name, "No recent tasks — recorded empty scrape");
            return Ok((0, scrape_id));
        }

        // Step 3c: Get seen task IDs from last scrape
        let seen_ids = db::task::get_seen_task_ids(
            &self.pool, model_name,
        ).await?;

        // Filter out seen tasks
        let new_records: Vec<&crate::parse::TaskRecord> = recent_tasks.iter()
            .filter(|t| !seen_ids.contains(&t.task_id))
            .collect();

        if new_records.is_empty() {
            // All tasks are duplicates — still record scrape
            let scrape_id = db::scrape::insert(&self.pool, scrape_insert).await?;
            return Ok((0, scrape_id));
        }

        // Build TaskInsert rows
        let task_inserts: Vec<TaskInsert> = new_records.iter()
            .map(|r| {
                let mut insert: TaskInsert = (*r).into();
                insert.model_load_id = model_load_id;
                insert.captured_at = scrape_insert.scraped_at;
                insert
            })
            .collect();

        // Step 3d: Insert new tasks
        let inserted = db::task::bulk_insert(&self.pool, &task_inserts).await?;

        // Step 3e: Insert scrape with aggregates and task IDs
        let scrape_id = db::scrape::insert(&self.pool, scrape_insert).await?;

        Ok((inserted as u64, scrape_id))
    }
}

/// Convert a `ModelLoad` into a `ModelLoadInsert` for the database.
fn model_load_to_insert<'a>(load: &'a ModelLoad, entry: &'a ModelEntry) -> ModelLoadInsert<'a> {
    ModelLoadInsert {
        model_name: &entry.model,
        created: entry.created,
        status: load.status.as_str(),
        preset_raw: Some(&load.preset.raw),
        ctx_size: load.preset.ctx_size,
        n_batch: None, // not available in preset — set from metrics later
        n_ubatch: load.preset.ubatch_size,
        parallel: load.preset.parallel,
        threads: load.preset.threads,
        threads_batch: load.preset.threads_batch,
        n_gpu_layers: load.preset.n_gpu_layers,
        cache_type_k: load.preset.cache_type_k.as_deref(),
        cache_type_v: load.preset.cache_type_v.as_deref(),
        flash_attn: load.preset.flash_attn.map(|b| if b { 1 } else { 0 }),
        kv_unified: load.preset.kv_unified.map(|b| if b { 1 } else { 0 }),
        context_bytes: None,
        model_bytes: None,
        cache_ram: load.preset.cache_ram,
        cache_reuse: load.preset.cache_reuse,
        checkpoint_min_step: load.preset.checkpoint_min_step,
        draft_p_min: load.preset.draft_p_min,
        image_min_tokens: load.preset.image_min_tokens,
        load_mode: load.preset.load_mode.as_deref(),
        op_offload: load.preset.op_offload.as_deref(),
        reasoning: load.preset.reasoning.map(|b| if b { 1 } else { 0 }),
        reasoning_budget: load.preset.reasoning_budget,
        reasoning_format: load.preset.reasoning_format.as_deref(),
        spec_draft_n_max: load.preset.spec_draft_n_max,
        spec_type: load.preset.spec_type.as_deref(),
        swa_checkpoints: load.preset.swa_checkpoints,
    }
}
