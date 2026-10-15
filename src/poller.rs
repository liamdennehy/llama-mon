//! Polling engine — orchestrates fetches, deduplication, reload
//! detection, and persistence.
//!
//! The poller runs a loop that:
//! 1. Fetches `/models` to discover loaded models
//! 2. Upserts each model's metadata in `model_loads`
//! 3. Fetches `/metrics` per model
//! 4. Detects reloads via `tasks.completed` counter decrease
//! 5. Deduplicates new tasks via `task_ids_seen` CSV
//! 6. Persists tasks and scrapes to SQLite

use thiserror::Error;
use tokio::sync::mpsc::UnboundedReceiver;

use crate::config::Config;
use crate::db;
use crate::db::model_load::{self, ModelLoadInsert};
use crate::db::task::TaskInsert;
use crate::http_client::HttpClient;
use crate::metrics_api;
use crate::models_api;
use crate::parse::{ModelEntry, ModelLoad};

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
    config: Config,
    pool: sqlx::SqlitePool,
    stats: PollStats,
    /// Tracks which model names were seen in the last `/models` poll.
    /// Used for lifecycle transitions (DB-only loads → unloaded).
    seen_models: std::collections::HashSet<String>,
}

impl Poller {
    /// Create a new poller.
    pub fn new(mut config: Config, pool: sqlx::SqlitePool) -> Self {
        let client = HttpClient::new(
            config.resolve().expect("invalid server URL"),
            config.timeout_duration(),
        );
        Self {
            client,
            config,
            pool,
            stats: PollStats::default(),
            seen_models: std::collections::HashSet::new(),
        }
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
                }
            }

            tokio::time::sleep(self.config.poll_duration()).await;
        }

        // Should not reach here, but handle gracefully
        tracing::info!("Poller loop ended unexpectedly");
        Err(PollerError::Stopped)
    }

    /// Execute a single poll cycle.
    ///
    /// 1. Fetch `/models` → upsert model loads
    /// 2. For each loaded model, fetch `/metrics`
    /// 3. Detect reloads, deduplicate, persist
    async fn poll_cycle(&mut self) -> Result<PollStats, PollerError> {
        let cycle_span = tracing::info_span!("poll_cycle");
        let _enter = cycle_span.enter();
        let cycle_start = std::time::Instant::now();

        // Step 1: Fetch /models
        let models_resp = match models_api::fetch_models(&self.client).await {
            Ok(resp) => resp,
            Err(e) => {
                match &e {
                    crate::http_client::HttpError::Status(code, _) if *code >= 400 && *code < 500 => {
                        tracing::error!(status = code, error = %e, "/models returned 4xx — skipping this cycle");
                    }
                    crate::http_client::HttpError::Status(code, _) if *code >= 500 => {
                        tracing::error!(status = code, error = %e, "/models returned 5xx — will retry next cycle");
                    }
                    _ => {
                        tracing::error!(error = %e, "Failed to fetch /models");
                    }
                }
                return Ok(self.stats.clone());
            }
        };
        self.seen_models.clear();

        // Edge case: empty data[]
        if models_resp.data.is_empty() {
            tracing::warn!("/models returned empty data[] — no active models");
        }

        let mut new_loads = 0u64;
        let mut active_models: Vec<(ModelLoad, ModelEntry)> = Vec::new();

        for entry in &models_resp.data {
            self.seen_models.insert(entry.model.clone());
            let load = ModelLoad::from_entry(entry);

            // Step 1b: Model filter — skip models not in the filter list
            if self.config.has_model_filter() {
                let filtered = self.config.models.iter().any(|m| {
                    m.eq_ignore_ascii_case(&entry.model)
                });
                if !filtered {
                    tracing::debug!(model = entry.model, "Skipped by model filter");
                    continue;
                }
            }

            // Log preset unknown keys warning (once per model load)
            if !load.preset.unknown_keys.is_empty() {
                let unknown: Vec<&str> = load.preset.unknown_keys.iter()
                    .map(|s| s.as_str())
                    .collect();
                tracing::warn!(
                    model = entry.model,
                    unknown_keys = ?unknown,
                    "Unknown preset keys detected (logged once per model load)"
                );
            }

            // Step 2: Upsert model load
            let load_insert = model_load_to_insert(&load, entry);
            let load_id = model_load::upsert(&self.pool, &load_insert).await?;

            // Step 3: Check for new loads vs existing
            let existing: Option<i64> = sqlx::query_scalar(
                "SELECT id FROM model_loads WHERE model_name = ? AND created = ?"
            )
            .bind(&entry.model)
            .bind(entry.created)
            .fetch_optional(&self.pool)
            .await.map_err(db::DbError::from)?;

            if existing.is_none() {
                new_loads += 1;
            }

            // Only process metrics for loaded/sleeping models
            if load.status.is_running() {
                let entry_clone = entry.clone();
                active_models.push((load, entry_clone));
            } else {
                // Non-running model — just update status and continue
                model_load::update_status(&self.pool, load_id, load.status.as_str(), None).await?;
            }
        }

        // Step 4: Lifecycle transitions — unload DB-only models
        let unloaded = self.check_lifecycle_transitions().await?;

        // Step 5: Fetch metrics and persist for each active model
        let mut total_new_tasks = 0u64;
        let mut total_scrapes = 0u64;

        for (load, entry) in &active_models {
            let load_insert = model_load_to_insert(load, entry);
            let load_id = model_load::upsert(&self.pool, &load_insert).await?;

            // Fetch metrics for this model
            let metrics_resp = match metrics_api::fetch_metrics(&self.client, &entry.model).await {
                Ok(resp) => resp,
                Err(e) => {
                    match &e {
                        crate::http_client::HttpError::Status(code, _) if *code >= 400 && *code < 500 => {
                            tracing::error!(model = entry.model, status = code, error = %e, "/metrics returned 4xx — skipping");
                        }
                        crate::http_client::HttpError::Status(code, _) if *code >= 500 => {
                            tracing::error!(model = entry.model, status = code, error = %e, "/metrics returned 5xx — will retry next cycle");
                        }
                        _ => {
                            tracing::error!(model = entry.model, error = %e, "Failed to fetch /metrics");
                        }
                    }
                    continue;
                }
            };

            // Extract metrics from first active model entry
            let active_metrics = match metrics_resp.active.first() {
                Some(m) => m,
                None => {
                    tracing::warn!(model = entry.model, "No active model in /metrics response");
                    continue;
                }
            };

            let full_metrics = &active_metrics.metrics;
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock went backwards")
                .as_secs() as i64;

            // Reload detection
            let reload_event = self.detect_reload(&entry.model, entry.created, full_metrics.tasks.completed).await?;

            match reload_event {
                ReloadEvent::ReloadDetected { .. } => {
                    // Close the old model load and create a new one
                    let old_id = load_id;
                    model_load::close(&self.pool, old_id, now).await?;
                    tracing::info!(
                        model = entry.model,
                        previous_completed = match reload_event {
                            ReloadEvent::ReloadDetected { previous_completed, .. } => previous_completed,
                            _ => 0,
                        },
                        "Model reload detected — closed old session"
                    );

                    // Create new model load
                    let new_created = entry.created + 1;
                    let new_load_insert = model_load_to_insert(&load, entry);
                    let new_load_id = model_load::upsert(&self.pool, &new_load_insert).await?;

                    // Build scrape insert from metrics
                    let mut scrape_insert: db::scrape::ScrapeInsert =
                        (full_metrics, now).into();
                    scrape_insert.model_name = entry.model.clone();
                    scrape_insert.model_created = new_created;
                    scrape_insert.model_load_id = new_load_id;

                    let (new_tasks, scrape_id) = self.process_metrics(
                        &entry.model, new_created, new_load_id,
                        &full_metrics.recent, &scrape_insert,
                    ).await?;
                    total_new_tasks += new_tasks;
                    total_scrapes += if scrape_id > 0 { 1 } else { 0 };
                }
                ReloadEvent::FirstScrape | ReloadEvent::SameSession { .. } => {
                    let mut scrape_insert: db::scrape::ScrapeInsert =
                        (full_metrics, now).into();
                    scrape_insert.model_name = entry.model.clone();
                    scrape_insert.model_created = entry.created;
                    scrape_insert.model_load_id = load_id;

                    let (new_tasks, scrape_id) = self.process_metrics(
                        &entry.model, entry.created, load_id,
                        &full_metrics.recent, &scrape_insert,
                    ).await?;
                    total_new_tasks += new_tasks;
                    total_scrapes += if scrape_id > 0 { 1 } else { 0 };
                }
            }
        }

        // Update scrape counts
        for model_name in &self.seen_models {
            let ids = model_load::get_ids_by_name(&self.pool, model_name).await.unwrap_or_default();
            for id in ids {
                model_load::increment_scrape_count(&self.pool, id).await?;
            }
        }

        // Update stats
        self.stats.total_scrapes += total_scrapes;
        self.stats.total_tasks += total_new_tasks;
        self.stats.total_model_loads += new_loads;
        self.stats.total_poll_cycles += 1;

        let elapsed = cycle_start.elapsed();

        // Verbose: log individual task records
        if self.config.verbose && !active_models.is_empty() {
            tracing::debug!(
                model_count = active_models.len(),
                "Verbose mode enabled — full cycle logging"
            );
        }

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

    /// Check for lifecycle transitions: models in DB but not in current /models
    /// response should be marked as unloaded.
    async fn check_lifecycle_transitions(&mut self) -> Result<u64, PollerError> {
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
            if !self.seen_models.contains(&row.model_name) {
                model_load::close(&self.pool, row.id, now).await?;
                unloaded += 1;
                tracing::info!(
                    model = row.model_name,
                    load_id = row.id,
                    "Model no longer in /models — marked unloaded"
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
        model_created: i64,
        current_completed: i64,
    ) -> Result<ReloadEvent, PollerError> {
        let last_completed = db::task::get_last_completed_counter(
            &self.pool, model_name, model_created,
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
        model_created: i64,
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
            &self.pool, model_name, model_created,
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
    }
}
