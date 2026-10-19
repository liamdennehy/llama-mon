//! Embedded dashboard HTML and static file serving.

/// The embedded dashboard HTML.
///
/// A single-page app with inline CSS and JavaScript, Chart.js loaded from CDN.
pub const DASHBOARD_HTML: &[u8] = include_bytes!("../../web/index.html");

/// Serve the main dashboard page.
pub async fn serve_dashboard() -> axum::response::Response {
    axum::response::Response::builder()
        .header(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")
        .body(axum::body::Body::from(DASHBOARD_HTML.to_vec()))
        .unwrap()
}
