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

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::SystemTime;

use anyhow::Result;
use clap::Parser;
use futures::StreamExt;
use signal_hook_tokio::Signals;
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
    let mut shutdown = Signals::new([signal_hook::consts::SIGINT, signal_hook::consts::SIGTERM])?;
    let shutdown_flag = AtomicBool::new(false);

    // Startup banner
    tracing::info!(
        "Server: {}  Interval: {:?}  Timeout: {:?}  Verbose: {}",
        server_url,
        config.poll_duration(),
        config.timeout_duration(),
        config.verbose
    );

    // Main loop — just initialize and report readiness
    // Full polling logic will be implemented in Phase 4.
    loop {
        if shutdown_flag.load(Ordering::Relaxed) {
            break;
        }
        tokio::select! {
            _ = shutdown.next() => {
                shutdown_flag.store(true, Ordering::Relaxed);
            }
            _ = tokio::time::sleep(config.poll_duration()) => {
                // TODO: implement poll cycle
            }
        }
    }

    // Graceful shutdown
    tracing::info!("llama-mon shutting down");
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("clock went backwards")
        .as_secs();
    tracing::info!(shutdown_time = now, "shutdown complete");

    Ok(())
}
