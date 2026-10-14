//! HTTP client wrapper for llama-server API calls.
//!
//! Provides typed methods for `/models` and `/metrics` with timeout,
//! retry, and logging support.

use reqwest::Client;
use serde::de::DeserializeOwned;
use std::time::Duration;
use thiserror::Error;
use url::Url;

use crate::parse::{MetricsResponse, ModelsResponse};

#[derive(Error, Debug)]
pub enum HttpError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("JSON parse error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("URL join error: {0}")]
    Url(#[from] url::ParseError),
    #[error("HTTP status: {0}")]
    Status(#[from] anyhow::Error),
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
            max_retries: 1,
        }
    }

    /// GET a JSON response from the server.
    async fn get_json<T: DeserializeOwned>(&self, path: &str) -> Result<T, HttpError> {
        let mut url = self.base_url.clone();
        url.set_path(path);

        tracing::debug!(url = %url, "Fetching");

        // Simple retry: attempt once, retry once on transient failure
        let mut attempt = 0;
        loop {
            match self.client.get(url.clone()).send().await {
                Ok(response) => {
                    let status = response.status();
                    if !status.is_success() {
                        let body = response.text().await.unwrap_or_default();
                        return Err(anyhow::anyhow!(
                            "HTTP {}: {}",
                            status.as_u16(),
                            body.chars().take(256).collect::<String>()
                        ).into());
                    }
                    let body = response.text().await?;
                    return Ok(serde_json::from_str(&body)?);
                }
                Err(e) if attempt < self.max_retries => {
                    attempt += 1;
                    tracing::warn!(error = %e, attempt, "Retry on transient error");
                    tokio::time::sleep(Duration::from_millis(500 * attempt as u64)).await;
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
