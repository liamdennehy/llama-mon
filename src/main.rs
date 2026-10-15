//! llama-mon — Real-time monitoring for llama.cpp inference servers.
//!
//! Polls a llama-server's `/models` and `/metrics` endpoints and stores
//! per-task performance data in SQLite for statistical analysis.

mod config;
mod db;
mod http_client;
mod metrics_api;
mod models_api;
mod parse;
mod poller;
mod preset;

use poller::Poller;

use anyhow::Result;
use clap::Parser;
use futures::StreamExt;
use tracing_subscriber::prelude::__tracing_subscriber_SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use crate::config::Config;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let mut config = Config::parse();
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

    let server_url = config.resolve()?;

    tracing::info!(
        server_url = %server_url,
        poll_interval = ?config.poll_duration(),
        db_path = %config.db_path.display(),
        "llama-mon starting"
    );

    // Open database
    let pool = db::open_pool(&config.db_path).await?;
    db::schema::init_db(&pool).await?;
    tracing::info!("Database initialized");

    // Set up shutdown signal (SIGINT + SIGTERM)
    let mut signals =
        signal_hook_tokio::Signals::new([signal_hook::consts::SIGINT, signal_hook::consts::SIGTERM])?;
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::broadcast::channel(1);

    // Spawn a task that forwards signals to the broadcast channel
    tokio::spawn(async move {
        while let Some(_sig) = signals.next().await {
            let _ = shutdown_tx.send(());
        }
    });

    // Startup banner
    tracing::info!(
        "Server: {}  Interval: {:?}  Timeout: {:?}  Verbose: {}",
        server_url,
        config.poll_duration(),
        config.timeout_duration(),
        config.verbose
    );

    // Create and run the poller
    let mut poller = Poller::new(config, pool);
    if let Err(e) = poller.run(shutdown_rx).await {
        tracing::error!(error = %e, "Poller stopped");
    }

    // Final stats
    tracing::info!("llama-mon shutting down");

    Ok(())
}
