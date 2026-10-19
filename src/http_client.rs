//! HTTP client wrapper for llama-server API calls.
//!
//! Provides typed methods for `/models` and `/metrics` with timeout,
//! retry with exponential backoff, and structured request logging.

use std::time::{Duration, Instant};

use reqwest::Client;
use serde::de::DeserializeOwned;
use thiserror::Error;
use url::Url;

use crate::parse::{MetricsResponse, ModelsResponse};

/// Which HTTP status codes are considered transient (retry-worthy).
const RETRYABLE_STATUS: &[u16] = &[408, 429, 500, 502, 503, 504];

#[derive(Error, Debug)]
pub enum HttpError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("JSON parse error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("HTTP status {0}: {1}")]
    Status(u16, String),
}

/// Indicates whether a failure should trigger a retry.
fn is_transient(e: &reqwest::Error) -> bool {
    e.is_timeout() || e.is_connect() || e.is_request()
}

/// Indicates whether an HTTP status code should be retried.
fn is_retryable_status(code: u16) -> bool {
    RETRYABLE_STATUS.contains(&code)
}

pub struct HttpClient {
    client: Client,
    base_url: Url,
    max_retries: u32,
}

impl HttpClient {
    /// Create a new HTTP client.
    pub fn new(base_url: Url, timeout: Duration) -> Self {
        Self {
            client: Client::builder()
                .timeout(timeout)
                .build()
                .expect("Failed to build HTTP client"),
            base_url,
            max_retries: 2, // 1 initial + 2 retries = 3 total attempts
        }
    }

    /// Return the base URL of the configured server.
    pub fn base_url(&self) -> &Url {
        &self.base_url
    }

    /// GET a JSON response from the server with retry and logging.
    async fn get_json<T: DeserializeOwned>(&self, path: &str) -> Result<T, HttpError> {
        let mut url = self.base_url.clone();
        url.set_path(path);
        Self::do_get_json(&self.client, &url, &mut 0u32, self.max_retries).await
    }

    /// GET a JSON response from a pre-built URL with retry and logging.
    async fn get_json_raw<T: DeserializeOwned>(&self, url: &Url) -> Result<T, HttpError> {
        Self::do_get_json(&self.client, url, &mut 0u32, self.max_retries).await
    }

    /// Internal: shared retry loop for GET JSON requests.
    async fn do_get_json<T: DeserializeOwned>(
        client: &Client,
        url: &Url,
        attempt: &mut u32,
        max_retries: u32,
    ) -> Result<T, HttpError> {
        let mut attempt = *attempt;
        loop {
            attempt += 1;
            let started = Instant::now();

            tracing::debug!(url = %url, attempt, "Fetching");

            match client.get(url.clone())
                .header("Accept", "application/json")
                .send().await {
                Ok(response) => {
                    let status = response.status();
                    let elapsed = started.elapsed();
                    tracing::debug!(
                        url = %url,
                        status = status.as_u16(),
                        duration_ms = elapsed.as_millis(),
                        "Response"
                    );

                    if status.is_success() {
                        let body_bytes = response.bytes().await?;
                        let body = String::from_utf8_lossy(&body_bytes);
                        let truncated = body.chars().take(256).collect::<String>();
                        tracing::debug!(url = %url, full_url = %url.as_str(), status = status.as_u16(), body_len = body.len(), response = truncated, "Raw JSON response");
                        return match serde_json::from_str::<T>(&body) {
                            Ok(v) => Ok(v),
                            Err(e) => {
                                tracing::error!(
                                    url = %url,
                                    error = %e,
                                    body_len = body.len(),
                                    body_preview = %truncated,
                                    "Failed to deserialize JSON"
                                );
                                Err(HttpError::Json(e))
                            }
                        };
                    }

                    // Handle retryable vs non-retryable status codes
                    let status_code = status.as_u16();
                    let body = response.text().await.unwrap_or_default();
                    let truncated = body.chars().take(256).collect::<String>();

                    if is_retryable_status(status_code) && attempt <= max_retries {
                        let backoff = Duration::from_millis(200 * attempt as u64);
                        tracing::warn!(
                            status = status_code,
                            attempt,
                            max_retries,
                            backoff_ms = backoff.as_millis(),
                            "Retryable HTTP status, retrying"
                        );
                        tokio::time::sleep(backoff).await;
                        continue;
                    }

                    return Err(HttpError::Status(status_code, truncated));
                }
                Err(e) if attempt <= max_retries && is_transient(&e) => {
                    let backoff = Duration::from_millis(200 * attempt as u64);
                    tracing::warn!(
                        error = %e,
                        attempt,
                        max_retries,
                        backoff_ms = backoff.as_millis(),
                        "Transient error, retrying"
                    );
                    tokio::time::sleep(backoff).await;
                    continue;
                }
                Err(e) => return Err(HttpError::Http(e)),
            }
        }
    }

    /// Fetch `/models` endpoint.
    ///
    /// No query parameters — the llama-server `/models` endpoint does not
    /// accept a `?model=` parameter. All model filtering is done client-side.
    pub async fn fetch_models(&self) -> Result<ModelsResponse, HttpError> {
        self.get_json("/models").await
    }

    /// Fetch `/metrics?model=all` endpoint.
    ///
    /// The live llama-server requires `model=all` when multiple models are
    /// configured. Returns aggregated metrics across all active models.
    pub async fn fetch_metrics(&self) -> Result<MetricsResponse, HttpError> {
        let mut url = self.base_url.clone();
        url.set_path("/metrics");
        url.set_query(Some("model=all"));
        self.get_json_raw(&url).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_url_construction() {
        let base: Url = "http://localhost:8080/".parse().unwrap();
        // Correct approach: set_path + set_query separately
        let mut url = base.clone();
        url.set_path("/metrics");
        url.set_query(Some("model=all"));
        assert_eq!(url.as_str(), "http://localhost:8080/metrics?model=all");

        // Wrong approach: set_path with embedded query string
        let mut url2 = base.clone();
        url2.set_path("/metrics?model=all");
        // The ? gets URL-encoded to %3F
        assert!(url2.as_str().contains("%3F"),
            "set_path with query string encodes ?: {}", url2.as_str());
    }

    #[test]
    fn test_is_retryable_status() {
        assert!(is_retryable_status(408));
        assert!(is_retryable_status(429));
        assert!(is_retryable_status(500));
        assert!(is_retryable_status(502));
        assert!(is_retryable_status(503));
        assert!(is_retryable_status(504));
        assert!(!is_retryable_status(400));
        assert!(!is_retryable_status(401));
        assert!(!is_retryable_status(403));
        assert!(!is_retryable_status(404));
        assert!(!is_retryable_status(200));
    }
}
