//! Deserialization types for the `/models` endpoint response.

use serde::Deserialize;

use crate::preset::PresetConfig;

/// Top-level response from `/models`.
#[derive(Debug, Deserialize)]
pub struct ModelsResponse {
    pub data: Vec<ModelEntry>,
    pub available: Vec<String>,
    pub wait: Vec<String>,
    pub error: Vec<String>,
}

/// A single model entry in the `data[]` array.
#[derive(Debug, Deserialize)]
pub struct ModelEntry {
    pub model: String,
    pub created: i64,
    pub status: String,
    #[serde(default, rename = "status_preset")]
    pub preset: Option<String>,
}

/// Model load status as reported by the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelLoadStatus {
    Loaded,
    Sleeping,
    Unloaded,
    Unknown,
}

impl From<&str> for ModelLoadStatus {
    fn from(s: &str) -> Self {
        match s {
            "loaded"     => ModelLoadStatus::Loaded,
            "sleeping"   => ModelLoadStatus::Sleeping,
            "unloaded"   => ModelLoadStatus::Unloaded,
            _            => ModelLoadStatus::Unknown,
        }
    }
}

impl ModelLoadStatus {
    /// Whether the model is accepting requests.
    pub fn is_ready(&self) -> bool {
        matches!(self, ModelLoadStatus::Loaded)
    }

    /// Whether the model is running (loaded, loading, or sleeping).
    pub fn is_running(&self) -> bool {
        matches!(self, ModelLoadStatus::Loaded | ModelLoadStatus::Sleeping)
    }

    /// Convert to a string suitable for database storage.
    pub fn as_str(&self) -> &'static str {
        match self {
            ModelLoadStatus::Loaded => "loaded",
            ModelLoadStatus::Sleeping => "sleeping",
            ModelLoadStatus::Unloaded => "unloaded",
            ModelLoadStatus::Unknown => "unknown",
        }
    }
}

/// Structured representation of a model load for the poller.
#[derive(Debug, Clone)]
pub struct ModelLoad {
    pub model_name: String,
    pub created: i64,
    pub status: ModelLoadStatus,
    pub preset: PresetConfig,
}

impl ModelLoad {
    /// Convert a raw `ModelEntry` into a parsed `ModelLoad`.
    pub fn from_entry(entry: &ModelEntry) -> Self {
        Self {
            model_name: entry.model.clone(),
            created: entry.created,
            status: entry.status.as_str().into(),
            preset: entry.preset.as_deref().map(PresetConfig::parse).unwrap_or_default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_status_from_str() {
        assert_eq!(ModelLoadStatus::from("loaded"), ModelLoadStatus::Loaded);
        assert_eq!(ModelLoadStatus::from("sleeping"), ModelLoadStatus::Sleeping);
        assert_eq!(ModelLoadStatus::from("unloaded"), ModelLoadStatus::Unloaded);
        assert_eq!(ModelLoadStatus::from("not_found"), ModelLoadStatus::Unknown);
        assert_eq!(ModelLoadStatus::from("downloading"), ModelLoadStatus::Unknown);
    }

    #[test]
    fn test_status_is_ready() {
        assert!(ModelLoadStatus::Loaded.is_ready());
        assert!(!ModelLoadStatus::Sleeping.is_ready());
        assert!(!ModelLoadStatus::Unloaded.is_ready());
    }

    #[test]
    fn test_status_is_running() {
        assert!(ModelLoadStatus::Loaded.is_running());
        assert!(ModelLoadStatus::Sleeping.is_running());
        assert!(!ModelLoadStatus::Unloaded.is_running());
    }

    #[test]
    fn test_status_as_str() {
        assert_eq!(ModelLoadStatus::Loaded.as_str(), "loaded");
        assert_eq!(ModelLoadStatus::Sleeping.as_str(), "sleeping");
        assert_eq!(ModelLoadStatus::Unloaded.as_str(), "unloaded");
        assert_eq!(ModelLoadStatus::Unknown.as_str(), "unknown");
    }

    #[test]
    fn test_model_load_from_entry() {
        let entry = ModelEntry {
            model: "test-model".to_string(),
            created: 1234567890,
            status: "loaded".to_string(),
            preset: Some("[test]\nctx-size = 8192\nthreads = 4".to_string()),
        };
        let ml = ModelLoad::from_entry(&entry);
        assert_eq!(ml.model_name, "test-model");
        assert_eq!(ml.created, 1234567890);
        assert_eq!(ml.status, ModelLoadStatus::Loaded);
        assert_eq!(ml.preset.ctx_size, Some(8192));
        assert_eq!(ml.preset.threads, Some(4));
    }

    #[test]
    fn test_model_load_from_entry_no_preset() {
        let entry = ModelEntry {
            model: "test-model".to_string(),
            created: 1234567890,
            status: "loaded".to_string(),
            preset: None,
        };
        let ml = ModelLoad::from_entry(&entry);
        assert_eq!(ml.preset.ctx_size, None);
        assert_eq!(ml.preset.raw, "");
    }
}
