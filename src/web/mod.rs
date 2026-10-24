//! Embedded web server for the llama-mon dashboard.
//!
//! Serves a single-page dashboard and a REST API endpoint
//! for scraping and task data from the SQLite database.
//!
//! Enabled when `--port` is provided.

pub mod handlers;
pub mod static_files;

use std::net::SocketAddr;

use axum::{Router, extract::Request, http::{HeaderValue, HeaderName}, response::IntoResponse, routing::get, middleware::from_fn};
use sqlx::SqlitePool;
use tokio::sync::broadcast;

/// Inject security headers on every response.
async fn security_headers(request: Request, next: axum::middleware::Next) -> impl IntoResponse {
    let mut response = next.run(request).await;

    // Content-Security-Policy — lock down resource loading.
    // No framing, no eval, no external scripts.
    let csp_value = "default-src 'none'; script-src 'self'; \
        style-src 'self' 'unsafe-inline'; img-src data:; \
        connect-src 'self'; base-uri 'self'; form-action 'self'";
    response.headers_mut().insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_str(csp_value).expect("valid CSP"),
    );

    // Prevent MIME-type sniffing
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );

    // Prevent clickjacking
    response.headers_mut().insert(
        header::X_FRAME_OPTIONS,
        HeaderValue::from_static("SAMEORIGIN"),
    );

    // Control referrer information
    response.headers_mut().insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("strict-origin-when-cross-origin"),
    );

    // Disable legacy XSS filter (CSP handles it)
    response.headers_mut().insert(
        header::X_XSS_PROTECTION,
        HeaderValue::from_static("0"),
    );

    // Permissions policy — disable sensitive device features
    response.headers_mut().insert(
        HeaderName::from_static("permissions-policy"),
        HeaderValue::from_static("camera=(), microphone=(), geolocation=()"),
    );

    // Prevent caching of text responses (API + dashboard HTML)
    let ct = response.headers().get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if ct.contains("text/") || ct.contains("application/json") {
        response.headers_mut().insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-store"),
        );
    }

    response
}

/// HTTP header constants (re-exported from http crate).
mod header {
    pub use http::header::*;
}

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
        .route("/chart.js", get(static_files::serve_chart_js))
        .route("/app.js", get(static_files::serve_app_js))
        .route("/health", get(handlers::health))
        .route("/api/status", get(handlers::api_status))
        .route("/api/stats/summary", get(handlers::api_summary))
        .route("/api/models", get(handlers::api_models))
        .route("/api/scrapes", get(handlers::api_scrapes))
        .route("/api/tasks", get(handlers::api_tasks))
        .layer(from_fn(security_headers))
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
