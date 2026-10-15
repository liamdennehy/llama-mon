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

        let mut attempt = 0u32;
        loop {
            attempt += 1;
            let started = Instant::now();

            tracing::debug!(url = %url, attempt, "Fetching");

            match self.client.get(url.clone()).send().await {
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
                        let body = response.text().await?;
                        // Log truncated raw body for verbose/debug mode
                        let truncated = body.chars().take(256).collect::<String>();
                        tracing::debug!(response = truncated, "Raw JSON response");
                        return Ok(serde_json::from_str(&body)?);
                    }

                    // Handle retryable vs non-retryable status codes
                    let status_code = status.as_u16();
                    let body = response.text().await.unwrap_or_default();
                    let truncated = body.chars().take(256).collect::<String>();

                    if is_retryable_status(status_code) && attempt <= self.max_retries {
                        let backoff = Duration::from_millis(200 * attempt as u64);
                        tracing::warn!(
                            status = status_code,
                            attempt,
                            max_retries = self.max_retries,
                            backoff_ms = backoff.as_millis(),
                            "Retryable HTTP status, retrying"
                        );
                        tokio::time::sleep(backoff).await;
                        continue;
                    }

                    return Err(HttpError::Status(status_code, truncated));
                }
                Err(e) if attempt <= self.max_retries && is_transient(&e) => {
                    let backoff = Duration::from_millis(200 * attempt as u64);
                    tracing::warn!(
                        error = %e,
                        attempt,
                        max_retries = self.max_retries,
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
    pub async fn fetch_models(&self) -> Result<ModelsResponse, HttpError> {
        self.get_json("/models").await
    }

    /// Fetch `/metrics?model=<name>` endpoint.
    pub async fn fetch_metrics(&self, model_name: &str) -> Result<MetricsResponse, HttpError> {
        let path = format!(
            "/metrics?model={}",
            percent_encoding::utf8_percent_encode(
                model_name,
                percent_encoding::NON_ALPHANUMERIC
            )
        );
        self.get_json(&path).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
