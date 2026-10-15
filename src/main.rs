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

use poller::{Poller, SignalAction};

use anyhow::Result;
use futures::StreamExt;
use tracing_subscriber::prelude::__tracing_subscriber_SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use crate::config::Config;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    // Load .env file first (silently), then parse CLI args
    let mut config = Config::load();
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

    // Open database
    let pool = db::open_pool(&config.db_path).await?;
    db::schema::init_db(&pool).await?;
    tracing::info!("Database initialized");

    // Signal handler task — dispatches to channel
    let (action_tx, action_rx) = tokio::sync::mpsc::unbounded_channel::<SignalAction>();
    let mut signals = signal_hook_tokio::Signals::new([
        signal_hook::consts::SIGINT,
        signal_hook::consts::SIGTERM,
        signal_hook::consts::SIGHUP,
        signal_hook::consts::SIGUSR1,
    ])?;
    tokio::spawn(async move {
        while let Some(sig) = signals.next().await {
            let action = match sig {
                signal_hook::consts::SIGINT | signal_hook::consts::SIGTERM => SignalAction::Shutdown,
                signal_hook::consts::SIGHUP => SignalAction::DumpConfig,
                signal_hook::consts::SIGUSR1 => SignalAction::DumpStats,
                _ => continue,
            };
            if action_tx.send(action).is_err() {
                break; // main dropped receiver
            }
        }
    });

    // Startup banner
    tracing::info!(
        server = %server_url,
        poll_interval = ?config.poll_duration(),
        timeout = ?config.timeout_duration(),
        verbose = config.verbose,
        model_filter = ?config.models,
        "llama-mon starting"
    );

    // Create and run the poller
    let mut poller = Poller::new(config, pool);
    loop {
        tokio::select! {
            result = poller.run(action_rx) => {
                if let Err(e) = result {
                    tracing::error!(error = %e, "Poller stopped");
                }
                break;
            }
        }
    }

    tracing::info!("llama-mon shutting down");
    Ok(())
}
