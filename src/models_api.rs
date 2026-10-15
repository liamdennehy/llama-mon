//! `/models` API wrapper.
//!
//! Fetches and logs the list of loaded models from llama-server.

use crate::http_client::HttpClient;
use crate::parse::ModelsResponse;

/// Fetch the list of models from llama-server's `/models` endpoint.
///
/// Logs the number of loaded models and their statuses.
pub async fn fetch_models(client: &HttpClient) -> Result<ModelsResponse, crate::http_client::HttpError> {
    let resp = client.fetch_models().await?;

    let loaded: Vec<&str> = resp.data.iter()
        .filter(|m| m.status == "loaded")
        .map(|m| m.model.as_str())
        .collect();

    tracing::info!(
        total = resp.data.len(),
        loaded = loaded.len(),
        available = resp.available.len(),
        waiting = resp.wait.len(),
        errors = resp.error.len(),
        loaded_models = ?loaded,
        "Fetched /models"
    );

    Ok(resp)
}
