//! Preset configuration parser.
//!
//! Converts the raw key=value block from llama-server's `status.preset`
//! into a structured `PresetConfig`.

/// Parsed preset configuration from a llama-server model entry.
#[derive(Debug, Default, Clone)]
pub struct PresetConfig {
    pub cache_prompt: Option<bool>,
    pub jinja: Option<bool>,
    pub log_colors: Option<bool>,
    pub log_file: Option<String>,
    pub batch_size: Option<i64>,
    pub ctx_size: Option<i64>,
    pub cache_type_k: Option<String>,
    pub cache_type_v: Option<String>,
    pub flash_attn: Option<bool>,
    pub kv_unified: Option<bool>,
    pub log_verbosity: Option<i64>,
    pub model_path: Option<String>,
    pub n_gpu_layers: Option<i64>,
    pub parallel: Option<i64>,
    pub threads: Option<i64>,
    pub threads_batch: Option<i64>,
    pub ubatch_size: Option<i64>,
    /// Raw input string, preserved for audit/debug.
    pub raw: String,
}

impl PresetConfig {
    /// Parse a raw preset key=value block into a structured config.
    ///
    /// Lines starting with `[` and ending with `]` are treated as section
    /// headers and skipped. Remaining lines are split on `=` into key/value.
    pub fn parse(raw: &str) -> Self {
        let mut config = Self::default();
        config.raw = raw.to_string();

        for line in raw.lines() {
            let line = line.trim();

            // Skip section headers like [Model-Name]
            if line.starts_with('[') && line.ends_with(']') {
                continue;
            }

            // Parse key = value
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let key = key.trim();
            let value = value.trim();

            match key {
                "cache-prompt"     => config.cache_prompt     = parse_bool(value),
                "jinja"            => config.jinja            = parse_bool(value),
                "log-colors"       => config.log_colors       = parse_bool(value),
                "log-file"         => config.log_file         = Some(value.to_string()),
                "batch-size"       => config.batch_size       = value.parse().ok(),
                "ctx-size"         => config.ctx_size         = value.parse().ok(),
                "cache-type-k"     => config.cache_type_k     = Some(value.to_string()),
                "cache-type-v"     => config.cache_type_v     = Some(value.to_string()),
                "flash-attn"       => config.flash_attn       = parse_bool(value),
                "kv-unified"       => config.kv_unified       = parse_bool(value),
                "log-verbosity"    => config.log_verbosity    = value.parse().ok(),
                "model"            => config.model_path       = Some(value.to_string()),
                "n-gpu-layers"     => config.n_gpu_layers     = value.parse().ok(),
                "parallel"         => config.parallel         = value.parse().ok(),
                "threads"          => config.threads          = value.parse().ok(),
                "threads-batch"    => config.threads_batch    = value.parse().ok(),
                "ubatch-size"      => config.ubatch_size      = value.parse().ok(),
                _ => (), // Ignore unknown keys
            }
        }

        config
    }
}

/// Parse a boolean value from a preset string.
/// Handles both `true`/`false` and `0`/`1`.
pub fn parse_bool(value: &str) -> Option<bool> {
    match value {
        "true" | "1" => Some(true),
        "false" | "0" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_full_preset() {
        let raw = r#"[example-model]
cache-prompt = true
jinja = true
log-colors = off
log-file = /var/log/llama-server/example-model.log
batch-size = 1024
ctx-size = 16384
cache-type-k = f16
cache-type-v = f16
flash-attn = true
kv-unified = true
log-verbosity = 2
model = ./example-model-Q6_K.gguf
n-gpu-layers = -1
parallel = 2
threads = 4
threads-batch = 12
ubatch-size = 256"#;

        let config = PresetConfig::parse(raw);
        assert_eq!(config.ctx_size, Some(16384));
        assert_eq!(config.parallel, Some(2));
        assert_eq!(config.threads, Some(4));
        assert_eq!(config.threads_batch, Some(12));
        assert_eq!(config.n_gpu_layers, Some(-1));
        assert_eq!(config.cache_type_k, Some("f16".to_string()));
        assert_eq!(config.cache_type_v, Some("f16".to_string()));
        assert_eq!(config.batch_size, Some(1024));
        assert_eq!(config.cache_prompt, Some(true));
        assert_eq!(config.jinja, Some(true));
        // "off" is not a recognized boolean value, so it stays None
        assert_eq!(config.log_colors, None);
        assert_eq!(config.flash_attn, Some(true));
        assert_eq!(config.kv_unified, Some(true));
        assert_eq!(config.log_verbosity, Some(2));
        assert_eq!(config.ubatch_size, Some(256));
        assert_eq!(config.model_path, Some("./example-model-Q6_K.gguf".to_string()));
        assert_eq!(config.log_file, Some("/var/log/llama-server/example-model.log".to_string()));
        assert!(config.raw.contains("[example-model]"));
    }

    #[test]
    fn test_parse_bool_variants() {
        assert_eq!(parse_bool("true"), Some(true));
        assert_eq!(parse_bool("false"), Some(false));
        assert_eq!(parse_bool("1"), Some(true));
        assert_eq!(parse_bool("0"), Some(false));
        assert_eq!(parse_bool("yes"), None);
        assert_eq!(parse_bool(""), None);
    }

    #[test]
    fn test_parse_empty_preset() {
        let config = PresetConfig::parse("");
        assert_eq!(config.ctx_size, None);
        assert_eq!(config.raw, "");
    }

    #[test]
    fn test_parse_section_header_only() {
        let config = PresetConfig::parse("[my-model]");
        assert_eq!(config.model_path, None);
        assert_eq!(config.raw, "[my-model]");
    }
}
