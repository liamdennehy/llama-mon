//! Embedded web server for the llama-mon dashboard.
//!
//! Serves a single-page dashboard and a REST API endpoint
//! for scraping and task data from the SQLite database.
//!
//! Enabled when `--port` is provided.

pub mod handlers;
pub mod static_files;

use std::net::SocketAddr;

use axum::{Router, extract::Request, response::IntoResponse, routing::get, middleware::from_fn};
use sqlx::SqlitePool;
use tokio::sync::broadcast;

/// Middleware: log requests that return non-2xx status codes.
async fn log_errors(request: Request, next: axum::middleware::Next) -> impl IntoResponse {
    let response = next.run(request).await;
    let status = response.status();
    if !status.is_success() {
        tracing::warn!(status = status.as_u16(), "Non-2xx response");
    }
    response
}

/// Start the web dashboard server.
///
/// Spawns an axum server on `addr` that serves the dashboard HTML,
/// the REST API, and a health check endpoint.
///
/// The server listens for `SignalAction::Shutdown` via a broadcast
/// channel and shuts down gracefully when received.
pub async fn serve(
    pool: SqlitePool,
    addr: SocketAddr,
    shutdown_tx: broadcast::Sender<()>,
) -> anyhow::Result<()> {
    let app = Router::new()
        .route("/", get(static_files::serve_dashboard))
        .route("/health", get(handlers::health))
        .route("/api/stats/summary", get(handlers::api_summary))
        .route("/api/models", get(handlers::api_models))
        .route("/api/scrapes", get(handlers::api_scrapes))
        .route("/api/tasks", get(handlers::api_tasks))
        .layer(from_fn(log_errors))
        .with_state(pool);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(%addr, "Web dashboard listening");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal(shutdown_tx))
        .await?;

    tracing::info!("Web dashboard stopped");
    Ok(())
}

/// Wait for a shutdown signal on the broadcast channel.
async fn shutdown_signal(shutdown_tx: broadcast::Sender<()>) {
    let mut rx = shutdown_tx.subscribe();
    let _ = rx.recv().await;
}
