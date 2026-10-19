//! Embedded dashboard HTML and static file serving.

/// The embedded dashboard HTML.
///
/// A single-page app with CSS in a <style> block and JS loaded from app.js.
pub const DASHBOARD_HTML: &[u8] = include_bytes!("../../web/index.html");

/// The embedded application JavaScript.
pub const APP_JS: &[u8] = include_bytes!("../../web/app.js");

/// Serve the main dashboard page.
pub async fn serve_dashboard() -> axum::response::Response {
    axum::response::Response::builder()
        .header(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")
        .body(axum::body::Body::from(DASHBOARD_HTML.to_vec()))
        .unwrap()
}

/// Serve the application JavaScript bundle.
pub async fn serve_app_js() -> axum::response::Response {
    axum::response::Response::builder()
        .header(axum::http::header::CONTENT_TYPE, "application/javascript; charset=utf-8")
        .body(axum::body::Body::from(APP_JS.to_vec()))
        .unwrap()
}
