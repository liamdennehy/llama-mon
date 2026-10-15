# llama-mon JSON Parsing Design

## Overview

Serde-based parsers handle the llama-server's JSON API responses: the `/models` endpoint and the `/metrics` endpoint. The preset configuration is a special case — a raw multi-line key=value string that must be parsed into individual fields.

## Models Endpoint Parser

### Input Structure (from `docs/models-output.md`)

```json
{
  "data": [
    {
      "model": "example-model",
      "created": 1791993598,
      "status": "loaded",
      "status_preset": "[example-model]\ncache-prompt = true\njinja = true\n..."
    }
  ],
  "available": [],
  "wait": [],
  "error": []
}
```

### Deserialization Types

```rust
/// Top-level response from /models
#[derive(Debug, Deserialize)]
struct ModelsResponse {
    data: Vec<ModelEntry>,
    available: Vec<String>,
    wait: Vec<String>,
    error: Vec<String>,
}

/// A single model entry in data[]
#[derive(Debug, Deserialize)]
struct ModelEntry {
    model: String,
    created: i64,
    status: String,
    #[serde(default)]
    status_preset: Option<String>,
}
```

### Status Mapping

```rust
impl From<&str> for ModelLoadStatus {
    fn from(s: &str) -> Self {
        match s {
            "loaded"      => ModelLoadStatus::Loaded,
            "sleeping"    => ModelLoadStatus::Sleeping,
            "unloaded"    => ModelLoadStatus::Unloaded,
            _             => ModelLoadStatus::Unknown,
        }
    }
}
```

### Preset String Parser

The `status_preset` field is a raw INI-style block:

```
[example-model]
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
ubatch-size = 256
```

Parsed into a structured config:

```rust
#[derive(Debug, Default)]
struct PresetConfig {
    cache_prompt: Option<bool>,
    jinja: Option<bool>,
    log_colors: Option<bool>,
    log_file: Option<String>,
    batch_size: Option<i64>,
    ctx_size: Option<i64>,
    cache_type_k: Option<String>,
    cache_type_v: Option<String>,
    flash_attn: Option<bool>,
    kv_unified: Option<bool>,
    log_verbosity: Option<i64>,
    model_path: Option<String>,
    n_gpu_layers: Option<i64>,
    parallel: Option<i64>,
    threads: Option<i64>,
    threads_batch: Option<i64>,
    ubatch_size: Option<i64>,
    raw: String,  // preserve original
}
```

Parser implementation — split on newlines, skip the header line (starts with `[`), parse each `key = value` line:

```rust
impl PresetConfig {
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
                "cache-prompt"       => config.cache_prompt       = value.parse().ok(),
                "jinja"              => config.jinja              = value.parse().ok(),
                "log-colors"         => config.log_colors         = value.parse().ok(),
                "log-file"           => config.log_file           = Some(value.to_string()),
                "batch-size"         => config.batch_size         = value.parse().ok(),
                "ctx-size"           => config.ctx_size           = value.parse().ok(),
                "cache-type-k"       => config.cache_type_k       = Some(value.to_string()),
                "cache-type-v"       => config.cache_type_v       = Some(value.to_string()),
                "flash-attn"         => config.flash_attn         = value.parse().ok(),
                "kv-unified"         => config.kv_unified         = value.parse().ok(),
                "log-verbosity"      => config.log_verbosity      = value.parse().ok(),
                "model"              => config.model_path         = Some(value.to_string()),
                "n-gpu-layers"       => config.n_gpu_layers       = value.parse().ok(),
                "parallel"           => config.parallel           = value.parse().ok(),
                "threads"            => config.threads            = value.parse().ok(),
                "threads-batch"      => config.threads_batch      = value.parse().ok(),
                "ubatch-size"        => config.ubatch_size        = value.parse().ok(),
                _ => (), // Ignore unknown keys
            }
        }

        config
    }
}
```

Boolean parsing handles both `true`/`false` and `0`/`1`.

## Metrics Endpoint Parser

### Input Structure (from `docs/metrics-output.md`)

```json
{
  "active": [
    {
      "model": "example-model",
      "status": "loaded",
      "metrics": {
        "context_max": 16384,
        "tasks": {
          "processing": 0,
          "queued": 0,
          "completed": 1,
          "duration_seconds": 3.1
        },
        "prompt": {
          "tokens_total": 44,
          "tokens_cached_total": 0,
          "tokens_processed": 44
        },
        "prediction": {
          "tokens_total": 10,
          "tokens_processed": 10
        },
        "decode": {
          "total": 10,
          "n_tokens_max": 53,
          "n_busy_slots": 10
        },
        "kvcache": {
          "active_tokens": 0
        },
        "slots": [],
        "memory": {
          "context_bytes": 1879048192,
          "model_bytes": 2959223072
        },
        "recent": [
          {
            "task_id": 12,
            "slot": 1,
            "n_prompt_cached": 0,
            "n_prompt_processed": 44,
            "n_gen": 10,
            "n_draft_tokens": 0,
            "n_draft_accepted": 0,
            "prompt_ms": 1490.8,
            "gen_ms": 1609.5,
            "prompt_tps": 29.5,
            "gen_tps": 5.6,
            "prompt_per_token_ms": 33.9,
            "gen_per_token_ms": 178.8
          }
        ]
      }
    }
  ],
  "available": [],
  "wait": [],
  "error": []
}
```

### Deserialization Types

```rust
#[derive(Debug, Deserialize)]
struct MetricsResponse {
    active: Vec<ModelMetrics>,
    available: Vec<String>,
    wait: Vec<String>,
    error: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ModelMetrics {
    model: String,
    status: String,
    metrics: FullMetrics,
}

#[derive(Debug, Deserialize)]
struct FullMetrics {
    context_max: i64,
    tasks: TaskCounts,
    prompt: PromptMetrics,
    prediction: PredictionMetrics,
    decode: DecodeMetrics,
    kvcache: KvCacheMetrics,
    slots: Vec<SlotMetrics>,
    memory: MemoryMetrics,
    recent: Vec<TaskRecord>,
}
```

Per-sub-metric structures:

```rust
#[derive(Debug, Deserialize)]
struct TaskCounts {
    processing: i64,
    queued: i64,
    completed: i64,
    duration_seconds: f64,
}

#[derive(Debug, Deserialize)]
struct PromptMetrics {
    tokens_total: i64,
    tokens_cached_total: i64,
    tokens_processed: i64,
}

#[derive(Debug, Deserialize)]
struct PredictionMetrics {
    tokens_total: i64,
    tokens_processed: i64,
}

#[derive(Debug, Deserialize)]
struct DecodeMetrics {
    total: i64,
    n_tokens_max: i64,
    n_busy_slots: i64,
}

#[derive(Debug, Deserialize)]
struct KvCacheMetrics {
    active_tokens: i64,
}

#[derive(Debug, Deserialize)]
struct MemoryMetrics {
    context_bytes: i64,
    model_bytes: i64,
}
```

### TaskRecord (from recent[])

```rust
#[derive(Debug, Deserialize)]
struct TaskRecord {
    task_id: i64,
    slot: i64,
    n_prompt_cached: i64,
    n_prompt_processed: i64,
    n_gen: i64,
    n_draft_tokens: i64,
    n_draft_accepted: i64,
    prompt_ms: f64,
    gen_ms: f64,
    prompt_tps: f64,
    gen_tps: f64,
    prompt_per_token_ms: f64,
    gen_per_token_ms: f64,
}
```

### SlotMetrics (from slots[], may be empty during idle)

```rust
#[derive(Debug, Deserialize)]
struct SlotMetrics {
    id: i64,
    // Additional fields as needed for active-slot diagnostics
}
```

### Aggregated Metrics for Scrapes Table

The `FullMetrics` struct is also the source for aggregate fields stored in `scrapes`:

| Scrapes Column | Source Field |
|---|---|
| `context_max` | `metrics.context_max` |
| `tasks_processing` | `metrics.tasks.processing` |
| `tasks_queued` | `metrics.tasks.queued` |
| `tasks_completed` | `metrics.tasks.completed` |
| `tasks_duration_s` | `metrics.tasks.duration_seconds` |
| `prompt_tokens_total` | `metrics.prompt.tokens_total` |
| `prompt_tokens_cached` | `metrics.prompt.tokens_cached_total` |
| `prompt_tokens_processed` | `metrics.prompt.tokens_processed` |
| `prediction_tokens_total` | `metrics.prediction.tokens_total` |
| `prediction_tokens_processed` | `metrics.prediction.tokens_processed` |
| `decode_total` | `metrics.decode.total` |
| `kvcache_active_tokens` | `metrics.kvcache.active_tokens` |
| `memory_context_bytes` | `metrics.memory.context_bytes` |
| `memory_model_bytes` | `metrics.memory.model_bytes` |

### Task Record Conversion

`TaskRecord` maps directly to the `tasks` table columns, with one transformation:

```rust
impl From<&TaskRecord> for TaskInsert {
    fn from(record: &TaskRecord) -> Self {
        Self {
            task_id: record.task_id,
            slot_id: record.slot,
            n_prompt_cached: record.n_prompt_cached,
            n_prompt_processed: record.n_prompt_processed,
            n_gen: record.n_gen,
            n_draft_tokens: record.n_draft_tokens,
            n_draft_accepted: record.n_draft_accepted,
            prompt_ms: record.prompt_ms,
            gen_ms: record.gen_ms,
            prompt_tps: record.prompt_tps,
            gen_tps: record.gen_tps,
            prompt_per_token_ms: record.prompt_per_token_ms,
            gen_per_token_ms: record.gen_per_token_ms,
        }
    }
}
```

## Model Reload Detection (Counter-Based)

Rather than relying solely on the `created` timestamp from `/models`, the scraper also monitors the cumulative `metrics.tasks.completed` counter from `/metrics`. This counter is monotonically increasing within a single model load session and resets (goes down) when the model is reloaded — whether explicitly or after an unload/reload cycle.

| Condition | Interpretation |
|---|---|
| `tasks.completed` increases from previous scrape | Same model load session, continue collecting tasks |
| `tasks.completed` stays the same | Model idle (sleeping) or no new completions yet |
| `tasks.completed` goes down | Model has been reloaded — this is a new load session |
| `tasks.completed` is absent or zero | Model may have just been loaded; treat as new load |

When a decrease is detected:
1. The current `model_load` row is closed (`unloaded_at = now`, `status = unloaded`)
2. A new `model_load` row is created (with the current `created` timestamp and preset)
3. The first scrape of the new session captures all currently-seen `recent[]` tasks
4. Subsequent scrapes use incremental dedup as normal

This also detects reloads with **different parameters** — if `preset_raw` (or individual preset fields like `ctx-size`) changes between the old and new session, the `model_loads` table preserves both sessions for comparison.

## Preset Boolean Parsing Details

Values in the preset can be:
- `true` / `false` — standard boolean
- `0` / `1` — numeric booleans (some preset values use this)

Parser handles both:

```rust
fn parse_bool(value: &str) -> Option<bool> {
    match value {
        "true" | "1" => Some(true),
        "false" | "0" => Some(false),
        _ => None,
    }
}
```

## Error Handling Strategy

| Scenario | Handling |
|---|---|
| Missing `status_preset` in /models response | `Option<String>` — set all preset fields to `None`, store `preset_raw` as empty |
| Unknown preset key | Ignored (match arm with `_ => ()`) |
| Non-numeric value in preset | `value.parse().ok()` returns `None`, field stays `None` |
| Empty `recent[]` array | No tasks to insert, continue with aggregate metrics only |
| Malformed JSON | `serde_json::Error` caught by caller, logged with response body snippet, poll cycle continues |
| Missing metric fields | Use `#[serde(default)]` on optional fields — missing fields get type defaults (0, 0.0, empty Vec) |
| `active[]` has 0 or multiple entries | Take `active.first()` if present; log warning if `active.len() > 1` (should not happen in practice) |

## Field Name Mapping (Preset Keys → Struct Fields)

Preset keys use kebab-case; struct fields use snake_case:

| Preset Key | Struct Field | Type |
|---|---|---|
| `cache-prompt` | `cache_prompt` | `bool` |
| `jinja` | `jinja` | `bool` |
| `log-colors` | `log_colors` | `bool` |
| `log-file` | `log_file` | `String` |
| `batch-size` | `batch_size` | `i64` |
| `ctx-size` | `ctx_size` | `i64` |
| `cache-type-k` | `cache_type_k` | `String` |
| `cache-type-v` | `cache_type_v` | `String` |
| `flash-attn` | `flash_attn` | `bool` |
| `kv-unified` | `kv_unified` | `bool` |
| `log-verbosity` | `log_verbosity` | `i64` |
| `model` | `model_path` | `String` |
| `n-gpu-layers` | `n_gpu_layers` | `i64` |
| `parallel` | `parallel` | `i64` |
| `threads` | `threads` | `i64` |
| `threads-batch` | `threads_batch` | `i64` |
| `ubatch-size` | `ubatch_size` | `i64` |

## Module Layout

```
src/
├── parse/
│   ├── mod.rs       // Re-exports: ModelsResponse, MetricsResponse, TaskRecord, etc.
│   ├── models.rs    // ModelsResponse, ModelEntry, PresetConfig, ModelLoadStatus
│   └── metrics.rs   // MetricsResponse, ModelMetrics, FullMetrics, TaskRecord, etc.
```

`mod.rs` re-exports the public types so callers use `llama_mon::parse::TaskRecord` rather than the internal path.
