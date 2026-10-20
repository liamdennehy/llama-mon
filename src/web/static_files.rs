//! Dashboard asset serving — embed at compile time by default, or serve from
//! the `web/` directory on-disk when compiled with the `dev` feature.

use std::path::PathBuf;

#[cfg(feature = "dev")]
const WEB_DIR: &str = "web";

#[cfg(feature = "dev")]
fn web_path(filename: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(WEB_DIR).join(filename)
}

/// Content type by file extension.
fn content_type(filename: &str) -> &'static str {
    match filename.rsplit('.').next().map(str::to_lowercase).as_deref() {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "application/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("png") => "image/png",
        Some("svg") => "image/svg+xml",
        _ => "application/octet-stream",
    }
}

/// Serve the main dashboard page.
pub async fn serve_dashboard() -> axum::response::Response {
    #[cfg(not(feature = "dev"))]
    {
        axum::response::Response::builder()
            .header(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")
            .body(axum::body::Body::from(DASHBOARD_HTML.to_vec()))
            .unwrap()
    }
    #[cfg(feature = "dev")]
    {
        serve_file("index.html").await
    }
}

/// Serve the application JavaScript bundle.
pub async fn serve_app_js() -> axum::response::Response {
    #[cfg(not(feature = "dev"))]
    {
        axum::response::Response::builder()
            .header(axum::http::header::CONTENT_TYPE, "application/javascript; charset=utf-8")
            .body(axum::body::Body::from(APP_JS.to_vec()))
            .unwrap()
    }
    #[cfg(feature = "dev")]
    {
        serve_file("app.js").await
    }
}

#[cfg(not(feature = "dev"))]
/// The embedded dashboard HTML.
pub const DASHBOARD_HTML: &[u8] = include_bytes!("../../web/index.html");

#[cfg(not(feature = "dev"))]
/// The embedded application JavaScript.
pub const APP_JS: &[u8] = include_bytes!("../../web/app.js");

/// Serve a file from the `web/` directory.
#[cfg(feature = "dev")]
pub async fn serve_file(filename: &str) -> axum::response::Response {
    let path = web_path(filename);
    match std::fs::read(&path) {
        Ok(contents) => axum::response::Response::builder()
            .header(axum::http::header::CONTENT_TYPE, content_type(filename))
            .body(axum::body::Body::from(contents))
            .unwrap(),
        Err(_) => axum::response::Response::builder()
            .status(axum::http::StatusCode::NOT_FOUND)
            .body(axum::body::Body::from("Not found"))
            .unwrap(),
    }
}
