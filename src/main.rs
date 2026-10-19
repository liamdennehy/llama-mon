//! llama-mon — Real-time monitoring for llama.cpp inference servers.
//!
//! Polls llama-server(s) `/models` and `/metrics` endpoints and stores
//! per-task performance data in SQLite for statistical analysis.
//!
//! Modes:
//! - **Full daemon** (default): `--server URL` + `--port PORT`
//! - **Scraper only**: `--server URL` (no `--port`)
//! - **Reporter only**: `--port PORT` (no `--server`)

use anyhow::{Context, Result};
use futures::StreamExt;
use tracing_subscriber::prelude::__tracing_subscriber_SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    // Load .env file first (silently), then parse CLI args
    let mut config = llama_mon::config::Config::load();
    config.resolve()?;

    // Initialize logging
    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_target(false)
        .with_level(true);
    let env_filter = if config.verbose {
        tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("debug"))
    } else {
        tracing_subscriber::EnvFilter::new("info")
    };
    tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt_layer)
        .init();

    // Shared shutdown channel — all pollers and the web server listen on this
    let (shutdown_tx, _) = tokio::sync::broadcast::channel(1);
    let shutdown_clone = shutdown_tx.clone();

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

    // Signal handler task — dispatches to channel
    let mut signals = signal_hook_tokio::Signals::new([
        signal_hook::consts::SIGINT,
        signal_hook::consts::SIGTERM,
        signal_hook::consts::SIGHUP,
        signal_hook::consts::SIGUSR1,
    ])?;
    tokio::spawn(async move {
        while let Some(sig) = signals.next().await {
            match sig {
                signal_hook::consts::SIGINT | signal_hook::consts::SIGTERM => {
                    let _ = shutdown_tx.send(());
                    break;
                }
                signal_hook::consts::SIGHUP => {
                    tracing::info!("SIGHUP received — re-reading config");
                }
                signal_hook::consts::SIGUSR1 => {
                    tracing::info!("SIGUSR1 received — stats dump would go here");
                }
                _ => continue,
            }
        }
    });

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
        let web_shutdown = shutdown_clone.clone();
        let h = tokio::spawn(llama_mon::web::serve(pool.clone(), addr, web_shutdown));
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
            let shutdown_rx = shutdown_clone.subscribe();
            let h = tokio::spawn(async move {
                poller.run_with_shutdown(shutdown_rx).await
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

    // Wait for any active component to finish.
    // When both are active, we wait for all pollers (they drive the lifecycle);
    // when only the web server runs, we wait for it.
    if !poll_handles.is_empty() {
        // Wait for each poller to finish
        for h in poll_handles {
            let result = h.await?;
            if let Err(e) = result {
                tracing::error!(error = %e, "Poller stopped");
            }
        }
        // All pollers finished — shut down the web server if running
        if let Some(h) = web_handle {
            let _ = h.await;
        }
    } else if let Some(h) = web_handle {
        // Only web server running — wait for it
        let _ = h.await;
    }

    tracing::info!("llama-mon shutting down");
    Ok(())
}
