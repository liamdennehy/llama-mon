//! `/metrics` API wrapper.
//!
//! Fetches and logs per-model metrics from llama-server.

use crate::http_client::HttpClient;
use crate::parse::MetricsResponse;

/// Fetch metrics for a specific model from llama-server's `/metrics` endpoint.
///
/// Logs the model name, number of active models, and number of recent tasks.
pub async fn fetch_metrics(
    client: &HttpClient,
    model_name: &str,
) -> Result<MetricsResponse, crate::http_client::HttpError> {
    let resp = client.fetch_metrics(model_name).await?;

    let total_recent: usize = resp.active.iter()
        .map(|m| m.metrics.recent.len())
        .sum();

    tracing::info!(
        model = model_name,
        active_models = resp.active.len(),
        available = resp.available.len(),
        waiting = resp.wait.len(),
        errors = resp.error.len(),
        total_recent_tasks = total_recent,
        "Fetched /metrics"
    );

    Ok(resp)
}
