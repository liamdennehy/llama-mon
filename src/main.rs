//! llama-mon — Real-time monitoring for llama.cpp inference servers.
//!
//! Polls llama-server(s) `/models` and `/metrics` endpoints and stores
//! per-task performance data in SQLite for statistical analysis.
//!
//! Modes:
//! - **Full daemon** (default): `--server URL` + `--port PORT`
//! - **Scraper only**: `--server URL` (no `--port`)
//! - **Reporter only**: `--port PORT` (no `--server`)
//!
//! Signal handling:
//! - **SIGINT / SIGTERM**: Graceful shutdown (WAL checkpoint, stop poller, stop web)
//! - **SIGUSR1**: Dump current model state to stdout as INFO log lines

use anyhow::{Context, Result};
use llama_mon::signals::SignalFlag;
use tracing_subscriber::prelude::__tracing_subscriber_SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    // Load .env file first (silently), then parse CLI args
    let mut config = llama_mon::config::Config::load();
    config.resolve()?;

    // Initialize logging based on verbosity level
    // RUST_LOG env var takes precedence over CLI flags when set.
    let has_rust_log = std::env::var("RUST_LOG").is_ok();

    match config.verbosity() {
        llama_mon::config::Verbosity::Silent if !has_rust_log => {
            // No logging at all
        }
        _ => {
            let fmt_layer = tracing_subscriber::fmt::layer()
                .with_target(false)
                .with_level(true);

            let filter = if has_rust_log {
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| {
                        tracing_subscriber::EnvFilter::new(config.verbosity().filter_directive().unwrap_or("warn"))
                    })
            } else {
                tracing_subscriber::EnvFilter::new(config.verbosity().filter_directive().unwrap_or("warn"))
            };

            tracing_subscriber::registry()
                .with(filter)
                .with(fmt_layer)
                .init();
        }
    }

    // Shared shutdown channel — pollers and web server listen on this
    let (shutdown_tx, _) = tokio::sync::broadcast::channel(1);

    // Open database for both scraper and reporter modes.
    // The scraper needs read-write access to persist data; the reporter
    // (no-scraper) needs read access to serve the dashboard.
    // In scraper mode, run `init_db` to create tables/views as needed.
    // In reporter mode, trust that the DB already exists and skip init.
    let pool = llama_mon::db::open_pool(&config.db_path).await
        .with_context(|| format!("Failed to open database at {}", config.db_path.display()))?;
    if config.should_scrub() {
        llama_mon::db::schema::init_db(&pool).await
            .with_context(|| format!("Failed to initialize database at {}", config.db_path.display()))?;
        tracing::info!(db_path = %config.db_path.display(), "Database initialized");
    } else {
        tracing::info!(db_path = %config.db_path.display(), "Database opened (reporter mode — no scraper)");
    }

    // Dedicated signal handler thread — sets atomic flags checked by the reactor.
    // Only SIGINT/SIGTERM and SIGUSR1 are handled; the rest are ignored.
    let signal_flag = SignalFlag::new();
    let _signal_thread = llama_mon::signals::spawn_handler(&signal_flag);

    // Startup banner
    let active_servers: Vec<String> = config
        .active_servers()
        .iter()
        .map(|s| s.identifier())
        .collect();
    let mode_parts: Vec<&str> = vec![
        if config.should_scrub() { "scraper" } else { "no-scraper" },
        if config.should_serve_web() { "web" } else { "no-web" },
    ];
    tracing::info!(
        servers = ?active_servers,
        poll_interval = ?config.poll_duration(),
        timeout = ?config.timeout_duration(),
        verbose = config.verbose,
        model_filter = ?config.models,
        fail_fast = config.fail_fast,
        mode = %mode_parts.join("+"),
        "llama-mon starting"
    );

    // Spawn optional web server (shared across all servers)
    let web_handle = if config.should_serve_web() {
        let port = config.web_port_effective();
        let addr = format!("{}:{}", config.listen_ip, port).parse().unwrap();
        let shutdown_clone = shutdown_tx.clone();
        let h = tokio::spawn(llama_mon::web::serve(pool.clone(), addr, shutdown_clone));
        tracing::info!(port, "Web dashboard enabled");
        Some(h)
    } else {
        tracing::info!("Web server disabled (no --port)");
        None
    };

    // Spawn one poller per active server
    let poll_handles: Vec<_> = if config.should_scrub() {
        let mut handles = Vec::new();
        for server in config.active_servers() {
            tracing::info!(server = %server.identifier(), "Starting poller");
            let mut poller = llama_mon::poller::Poller::new(
                server.clone(),
                config.clone(),
                pool.clone(),
            );
            let shutdown_clone = shutdown_tx.clone();
            let h = tokio::spawn(async move {
                poller.run_with_shutdown(shutdown_clone.subscribe()).await
            });
            handles.push(h);
        }
        if handles.is_empty() {
            tracing::warn!("No active servers configured — poller skipped");
        }
        handles
    } else {
        tracing::info!("Scraper disabled (no --server)");
        Vec::new()
    };

    // Reactor loop — checks the signal flag at well-defined boundaries.
    // The signal handler thread sets the atomic flag on signal arrival; this
    // loop polls it between await points to stay responsive.
    'reactor_loop: loop {
        // Drain all pending signals.
        let mut got_sigusr1 = false;
        loop {
            match signal_flag.take() {
                Some(1) => {
                    // Shutdown — send broadcast and exit immediately
                    tracing::info!("Shutdown signal received");
                    let _ = shutdown_tx.send(());
                    break 'reactor_loop;
                }
                Some(2) => got_sigusr1 = true,
                None => break,
                _ => {}
            }
        }

        if got_sigusr1 {
            dump_model_state(&pool).await;
        }

        let any_active = !poll_handles.is_empty() || web_handle.is_some();
        if !any_active {
            break;
        }

        if !poll_handles.is_empty() {
            // Check if any poller is still running
            if poll_handles.iter().any(|h| !h.is_finished()) {
                // Wait for all pollers (they will receive the shutdown signal
                // and exit gracefully)
                for h in poll_handles {
                    let result = h.await?;
                    if let Err(e) = result {
                        tracing::error!(error = %e, "Poller stopped");
                    }
                }
                if let Some(h) = web_handle {
                    let _ = h.await;
                }
                break;
            }
        } else if let Some(h) = web_handle {
            let _ = h.await;
            break;
        } else {
            break;
        }
    }

    tracing::info!("llama-mon shutting down");
    Ok(())
}

/// Dump the current state of all model loads from the database as INFO log lines.
async fn dump_model_state(pool: &sqlx::SqlitePool) {
    match llama_mon::db::model_load::get_all(pool).await {
        Ok(rows) => {
            if rows.is_empty() {
                tracing::info!("SIGUSR1: No model loads in database");
            } else {
                tracing::info!(
                    "SIGUSR1: Dumping {} model load(s)",
                    rows.len()
                );
                for row in &rows {
                    tracing::info!(
                        "MODEL: name={}, status={}, created={}, loaded_at={}, unloaded_at={}",
                        row.model_name,
                        row.status,
                        row.created,
                        row.loaded_at.as_ref().map(|v| v.to_string()).unwrap_or_else(|| "—".into()),
                        row.unloaded_at.as_ref().map(|v| v.to_string()).unwrap_or_else(|| "—".into()),
                    );
                }
            }
        }
        Err(e) => {
            tracing::error!(error = %e, "SIGUSR1: Failed to read model loads from database");
        }
    }
}
