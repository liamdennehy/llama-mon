//! `/metrics` API wrapper.
//!
//! Fetches and logs aggregated metrics from llama-server's `/metrics` endpoint.
//! The live server requires `model=all` for aggregated metrics across all models.

use crate::http_client::HttpClient;
use crate::parse::MetricsResponse;

/// Fetch aggregated metrics from llama-server's `/metrics` endpoint.
///
/// Logs the number of active models, available models, and total recent tasks.
pub async fn fetch_metrics(
    client: &HttpClient,
) -> Result<MetricsResponse, crate::http_client::HttpError> {
    let resp = client.fetch_metrics().await?;

    let total_recent: usize = resp.active.iter()
        .map(|m| m.metrics.recent.len())
        .sum();

    tracing::info!(
        active_models = resp.active.len(),
        available = resp.available.len(),
        waiting = resp.wait.len(),
        errors = resp.error.len(),
        total_recent_tasks = total_recent,
        "Fetched /metrics"
    );

    Ok(resp)
}
