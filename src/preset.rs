//! Preset configuration parser.
//!
//! Converts the raw key=value block from llama-server's `status.preset`
//! into a structured `PresetConfig`.

/// All preset keys emitted by llama-server's `/models` endpoint.
/// Keys are split into two groups: those llama-mon tracks in the DB
/// and those it intentionally ignores (logging, file paths, etc.).
const KNOWN_KEYS: &[&str] = &[
    // Tracked — stored in model_loads preset columns
    "batch-size", "ctx-size", "cache-type-k", "cache-type-v",
    "n-gpu-layers", "parallel", "threads", "threads-batch", "ubatch-size",
    "flash-attn", "kv-unified", "jinja", "cache-prompt",
    // Tracked — speculative decoding and serving knobs
    "cache-ram", "cache-reuse", "checkpoint-min-step",
    "draft-p-min", "image-min-tokens", "load-mode", "op-offload",
    "reasoning", "reasoning-budget", "reasoning-format",
    "spec-draft-n-max", "spec-type", "swa-checkpoints",
    // Ignored — known to exist but not relevant for performance analysis
    "model", "log-verbosity", "log-colors", "log-file",
];

/// Parsed preset configuration from a llama-server model entry.
#[derive(Debug, Default, Clone)]
pub struct PresetConfig {
    pub batch_size: Option<i64>,
    pub ctx_size: Option<i64>,
    pub cache_type_k: Option<String>,
    pub cache_type_v: Option<String>,
    pub n_gpu_layers: Option<i64>,
    pub parallel: Option<i64>,
    pub threads: Option<i64>,
    pub threads_batch: Option<i64>,
    pub ubatch_size: Option<i64>,
    /// Boolean preset knobs that affect serving behavior.
    pub flash_attn: Option<bool>,
    pub kv_unified: Option<bool>,
    pub jinja: Option<bool>,
    pub cache_prompt: Option<bool>,
    // Speculative decoding and serving knobs
    pub cache_ram: Option<i64>,
    pub cache_reuse: Option<i64>,
    pub checkpoint_min_step: Option<i64>,
    pub draft_p_min: Option<f64>,
    pub image_min_tokens: Option<i64>,
    pub load_mode: Option<String>,
    pub op_offload: Option<String>,
    pub reasoning: Option<bool>,
    pub reasoning_budget: Option<i64>,
    pub reasoning_format: Option<String>,
    pub spec_draft_n_max: Option<i64>,
    pub spec_type: Option<String>,
    pub swa_checkpoints: Option<i64>,
    /// Keys emitted by the server that are not recognized by llama-mon
    /// at all. Used to emit a one-per-session warning for audit.
    pub unknown_keys: std::collections::BTreeSet<String>,
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
                "batch-size"       => config.batch_size       = value.parse().ok(),
                "ctx-size"         => config.ctx_size         = value.parse().ok(),
                "cache-type-k"     => config.cache_type_k     = Some(value.to_string()),
                "cache-type-v"     => config.cache_type_v     = Some(value.to_string()),
                "flash-attn"       => config.flash_attn       = parse_bool(value),
                "kv-unified"       => config.kv_unified       = parse_bool(value),
                "n-gpu-layers"     => config.n_gpu_layers     = value.parse().ok(),
                "parallel"         => config.parallel         = value.parse().ok(),
                "threads"          => config.threads          = value.parse().ok(),
                "threads-batch"    => config.threads_batch    = value.parse().ok(),
                "ubatch-size"      => config.ubatch_size      = value.parse().ok(),
                // Speculative decoding and serving knobs
                "cache-ram"           => config.cache_ram           = value.parse().ok(),
                "cache-reuse"         => config.cache_reuse         = value.parse().ok(),
                "checkpoint-min-step" => config.checkpoint_min_step = value.parse().ok(),
                "draft-p-min"         => config.draft_p_min         = value.parse().ok(),
                "image-min-tokens"    => config.image_min_tokens    = value.parse().ok(),
                "load-mode"           => config.load_mode           = Some(value.to_string()),
                "op-offload"          => config.op_offload          = Some(value.to_string()),
                "reasoning"           => config.reasoning           = parse_bool(value),
                "reasoning-budget"    => config.reasoning_budget    = value.parse().ok(),
                "reasoning-format"    => config.reasoning_format    = Some(value.to_string()),
                "spec-draft-n-max"    => config.spec_draft_n_max    = value.parse().ok(),
                "spec-type"           => config.spec_type           = Some(value.to_string()),
                "swa-checkpoints"     => config.swa_checkpoints     = value.parse().ok(),
                // Intentionally ignored — known keys not stored in DB
                "model" | "log-verbosity" | "log-colors" | "log-file" => {},
                // Truly unknown — not in KNOWN_KEYS at all
                _ if KNOWN_KEYS.contains(&key) => {
                    unreachable!("{} is in KNOWN_KEYS but not matched above", key);
                }
                _ => {
                    config.unknown_keys.insert(key.to_string());
                }
            }
        }

        config
    }
}

/// Parse a boolean value from a preset string.
/// Handles `true`/`false`, `1`/`0`, and `on`/`off`.
pub fn parse_bool(value: &str) -> Option<bool> {
    match value {
        "true" | "1" | "on" => Some(true),
        "false" | "0" | "off" => Some(false),
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
        assert_eq!(config.flash_attn, Some(true));
        assert_eq!(config.kv_unified, Some(true));
        assert_eq!(config.ubatch_size, Some(256));
        // Ignored keys are not stored
        assert!(config.unknown_keys.is_empty());
        assert!(config.raw.contains("[example-model]"));
    }

    #[test]
    fn test_unknown_keys_collected() {
        let raw = "[test]\nfoo-bar = baz\nctx-size = 1024\nunknown-key = 42";
        let config = PresetConfig::parse(raw);
        assert_eq!(config.ctx_size, Some(1024));
        assert_eq!(config.unknown_keys.len(), 2);
        assert!(config.unknown_keys.contains("foo-bar"));
        assert!(config.unknown_keys.contains("unknown-key"));
    }

    #[test]
    fn test_parse_bool_variants() {
        assert_eq!(parse_bool("true"), Some(true));
        assert_eq!(parse_bool("false"), Some(false));
        assert_eq!(parse_bool("1"), Some(true));
        assert_eq!(parse_bool("0"), Some(false));
        assert_eq!(parse_bool("on"), Some(true));
        assert_eq!(parse_bool("off"), Some(false));
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
        assert!(config.unknown_keys.is_empty());
        assert_eq!(config.raw, "[my-model]");
    }

    #[test]
    fn test_parse_speculative_decoding_keys() {
        let raw = r#"[speculative]
draft-p-min = 0.1
spec-type = greedy
spec-draft-n-max = 8
reasoning = true
reasoning-budget = 512
reasoning-format = plaintext
cache-ram = 4096
cache-reuse = 16
checkpoint-min-step = 10
image-min-tokens = 4
load-mode = mmap
op-offload = cpu
swa-checkpoints = 4"#;
        let config = PresetConfig::parse(raw);
        assert_eq!(config.draft_p_min, Some(0.1));
        assert_eq!(config.spec_type, Some("greedy".to_string()));
        assert_eq!(config.spec_draft_n_max, Some(8));
        assert_eq!(config.reasoning, Some(true));
        assert_eq!(config.reasoning_budget, Some(512));
        assert_eq!(config.reasoning_format, Some("plaintext".to_string()));
        assert_eq!(config.cache_ram, Some(4096));
        assert_eq!(config.cache_reuse, Some(16));
        assert_eq!(config.checkpoint_min_step, Some(10));
        assert_eq!(config.image_min_tokens, Some(4));
        assert_eq!(config.load_mode, Some("mmap".to_string()));
        assert_eq!(config.op_offload, Some("cpu".to_string()));
        assert_eq!(config.swa_checkpoints, Some(4));
        // None of these should be in unknown_keys
        assert!(config.unknown_keys.is_empty());
    }
}
