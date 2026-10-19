# llama-mon Database Schema

## Overview

SQLite database storing model load instances and their completed tasks for statistical analysis. Designed for a single-process writer (no concurrent access), with read-only queries from the future dashboard.

## Schema Version

A single row in the `meta` table tracks schema version for future migrations.

```sql
CREATE TABLE IF NOT EXISTS meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

-- Initial row inserted on first connect:
-- INSERT INTO meta (key, value) VALUES ('version', '3');
```

## Tables

### model_loads

One row per invocation of a model on the llama-server. Each `(model_name, created)` pair is unique — a model reloaded after unload creates a new row.

```sql
CREATE TABLE IF NOT EXISTS model_loads (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    model_name        TEXT    NOT NULL,          -- e.g. "example-model"
    created           INTEGER NOT NULL,          -- unix timestamp from /models .data[].created
    status            TEXT    NOT NULL DEFAULT 'unknown',  -- loaded, sleeping, unloaded
    loaded_at         INTEGER,                   -- unix timestamp when first observed as "loaded"
    unloaded_at       INTEGER,                   -- unix timestamp when observed as "unloaded" or last seen
    preset_raw        TEXT,                      -- raw key=value string from status.preset
    ctx_size          INTEGER,                   -- derived from preset: ctx-size
    n_batch           INTEGER,                   -- derived from preset: batch-size
    n_ubatch          INTEGER,                   -- derived from preset: ubatch-size
    parallel          INTEGER,                   -- derived from preset: parallel (max slots)
    threads           INTEGER,                   -- derived from preset: threads
    threads_batch     INTEGER,                   -- derived from preset: threads-batch
    n_gpu_layers      INTEGER,                   -- derived from preset: n-gpu-layers
    cache_type_k      TEXT,                      -- derived from preset: cache-type-k
    cache_type_v      TEXT,                      -- derived from preset: cache-type-v
    model_path        TEXT,                      -- derived from preset: model (file path)
    log_verbosity     INTEGER,                   -- derived from preset: log-verbosity
    flash_attn        INTEGER,                   -- derived from preset: flash-attn (0/1)
    kv_unified        INTEGER,                   -- derived from preset: kv-unified (0/1)
    context_bytes     INTEGER,                   -- from metrics.memory.context_bytes
    model_bytes       INTEGER,                   -- from metrics.memory.model_bytes
    scrape_count      INTEGER NOT NULL DEFAULT 0, -- number of scrapes that have referenced this load
    -- Speculative decoding and serving knobs
    cache_ram         INTEGER,                   -- derived from preset: cache-ram (MB)
    cache_reuse       INTEGER,                   -- derived from preset: cache-reuse (ratio)
    checkpoint_min_step INTEGER,                 -- derived from preset: checkpoint-min-step
    draft_p_min       REAL,                      -- derived from preset: draft-p-min
    image_min_tokens  INTEGER,                   -- derived from preset: image-min-tokens
    load_mode         TEXT,                      -- derived from preset: load-mode
    op_offload        TEXT,                      -- derived from preset: op-offload
    reasoning         INTEGER,                   -- derived from preset: reasoning (0/1)
    reasoning_budget  INTEGER,                   -- derived from preset: reasoning-budget
    reasoning_format  TEXT,                      -- derived from preset: reasoning-format
    spec_draft_n_max  INTEGER,                   -- derived from preset: spec-draft-n-max
    spec_type         TEXT,                      -- derived from preset: spec-type
    swa_checkpoints   INTEGER                    -- derived from preset: swa-checkpoints
);

CREATE INDEX IF NOT EXISTS idx_model_loads_model_created
    ON model_loads (model_name, created);
```

#### Speculative decoding and serving knobs (schema v3)

Added in schema v3, these columns capture preset parameters related to
speculative decoding, KV cache management, and reasoning models. They
are parsed from the raw preset string and stored alongside existing
preset columns for performance correlation.

| Column | Preset key | Type | Notes |
|---|---|---|---|
| `cache_ram` | `cache-ram` | INTEGER | KV cache memory allocation in MB |
| `cache_reuse` | `cache-reuse` | INTEGER | KV cache reuse ratio |
| `checkpoint_min_step` | `checkpoint-min-step` | INTEGER | Minimum step for checkpointing |
| `draft_p_min` | `draft-p-min` | REAL | Minimum draft probability for speculative decoding |
| `image_min_tokens` | `image-min-tokens` | INTEGER | Minimum tokens for image processing |
| `load_mode` | `load-mode` | TEXT | File path for model loading mode |
| `op_offload` | `op-offload` | TEXT | File path for operator offloading |
| `reasoning` | `reasoning` | INTEGER | Whether reasoning mode is enabled (0/1) |
| `reasoning_budget` | `reasoning-budget` | INTEGER | Token budget for reasoning |
| `reasoning_format` | `reasoning-format` | TEXT | Format of reasoning output |
| `spec_draft_n_max` | `spec-draft-n-max` | INTEGER | Maximum draft tokens for speculative decoding |
| `spec_type` | `spec-type` | TEXT | Type of speculative decoding |
| `swa_checkpoints` | `swa-checkpoints` | INTEGER | Number of SWA (sliding window attention) checkpoints |

### tasks

One row per completed task, extracted from the `metrics.recent[]` array. Each task belongs to exactly one model load.

```sql
CREATE TABLE IF NOT EXISTS tasks (
    task_id           INTEGER NOT NULL,          -- task_id from server metrics.recent[].task_id
    model_load_id     INTEGER NOT NULL REFERENCES model_loads(id),
    slot_id           INTEGER NOT NULL,          -- slot number from metrics.recent[].slot

    -- Prompt metrics
    n_prompt_cached       INTEGER DEFAULT 0,
    n_prompt_processed    INTEGER NOT NULL,
    prompt_ms             REAL    NOT NULL,      -- total prompt processing time (ms)
    prompt_tps            REAL,                  -- prompt tokens per second
    prompt_per_token_ms   REAL,                  -- ms per prompt token

    -- Generation metrics
    n_gen             INTEGER NOT NULL,          -- total tokens generated
    gen_ms            REAL    NOT NULL,          -- total generation time (ms)
    gen_tps           REAL,                  -- generated tokens per second
    gen_per_token_ms  REAL,                  -- ms per generated token

    -- Speculative decoding (optional, 0 if not used)
    n_draft_tokens      INTEGER DEFAULT 0,
    n_draft_accepted    INTEGER DEFAULT 0,

    -- Capture metadata
    captured_at         INTEGER NOT NULL         -- unix timestamp of the scrape that captured this task
);

CREATE INDEX IF NOT EXISTS idx_tasks_model_load
    ON tasks (model_load_id);

CREATE INDEX IF NOT EXISTS idx_tasks_slot
    ON tasks (model_load_id, slot_id);

-- Unique constraint prevents duplicate insertion from overlapping scrapes
CREATE UNIQUE INDEX IF NOT EXISTS idx_tasks_unique
    ON tasks (task_id, model_load_id);
```

### scrapes

Tracks each poll of the `/metrics` endpoint. Enables incremental deduplication — only tasks with `task_id` values not seen in any prior scrape for that model load are inserted.

The `tasks_completed` column is the key to **reload detection**: when this value decreases from one scrape to the next, the model has been reloaded. The scraper closes the old `model_loads` row and begins a new one; this `scrapes` row belongs to the old session.

```sql
CREATE TABLE IF NOT EXISTS scrapes (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    model_name        TEXT    NOT NULL,          -- model queried
    model_created     INTEGER NOT NULL,          -- created timestamp of the model load
    model_load_id     INTEGER NOT NULL REFERENCES model_loads(id),
    scraped_at        INTEGER NOT NULL,          -- unix timestamp of this scrape

    -- Compact snapshot of aggregate metrics (from the top-level metrics object, not per-task)
    context_max       INTEGER,
    tasks_processing  INTEGER DEFAULT 0,
    tasks_queued      INTEGER DEFAULT 0,
    tasks_completed   INTEGER DEFAULT 0,
    tasks_duration_s  REAL,
    prompt_tokens_total   INTEGER,
    prompt_tokens_cached  INTEGER,
    prompt_tokens_processed INTEGER,
    prediction_tokens_total   INTEGER,
    prediction_tokens_processed INTEGER,
    decode_total      INTEGER,
    kvcache_active_tokens INTEGER,
    memory_context_bytes  INTEGER,
    memory_model_bytes    INTEGER,

    -- All task_ids seen in this scrape (compact storage for dedup)
    -- Format: comma-separated list of integers, e.g. "12,15,18"
    task_ids_seen TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_scrapes_model_created
    ON scrapes (model_name, model_created, scraped_at);
```

## Views

Views are created automatically on database initialization. They simplify common queries and provide pre-joined analyses.

### `v_task_summary`

Flattened task records with model metadata and computed context length. Replaces the `tasks JOIN model_loads` query used by the dashboard and CLI.

```sql
SELECT
    task_id, model_name, model_ctx_size, model_threads, model_parallel, model_created,
    slot_id, n_prompt_cached, n_prompt_processed, context_length,
    prompt_ms, prompt_tps, prompt_per_token_ms,
    n_gen, gen_ms, gen_tps, gen_per_token_ms,
    n_draft_tokens, n_draft_accepted, captured_at
FROM v_task_summary
WHERE model_name = 'Qwen3.6-35B-A3B'
  AND captured_at >= datetime('now', '-6 hours')
ORDER BY captured_at DESC;
```

### `v_scrape_summary`

Scrape snapshots joined with model metadata. Used by the dashboard time-series charts and CLI trend queries.

```sql
SELECT model_name, scraped_at, tasks_completed, context_max,
       memory_context_bytes / 1e9 AS mem_gb, model_ctx_size
FROM v_scrape_summary
WHERE model_name = 'Qwen3.6-35B-A3B'
  AND scraped_at >= datetime('now', '-1 day')
ORDER BY scraped_at;
```

### `v_model_stats`

Per-model-load performance aggregates. Answers questions like "which config gives the best TPS?"

```sql
SELECT model_name, model_ctx_size, model_threads, model_parallel,
       avg_gen_tps, p95_gen_tps, median_gen_tps,
       avg_prompt_tps, task_count, total_gen_tokens, scrape_count
FROM v_model_stats
ORDER BY avg_gen_tps DESC;
```

### `v_model_history`

Model load timeline with session durations and task/scrape counts. Answers "how long did each load last and how many tasks did it process?"

```sql
SELECT model_name, created, status, model_ctx_size,
       session_duration_s, task_count, scrape_count
FROM v_model_history
WHERE status = 'unloaded'
ORDER BY created DESC;
```

## Index Summary

| Index | Table | Purpose |
|-------|-------|---------|
| `idx_model_loads_model_created` | model_loads | Lookup loads by model name + recency |
| `idx_tasks_model_load` | tasks | Query all tasks for a model load |
| `idx_tasks_slot` | tasks | Per-slot performance analysis |
| `idx_tasks_unique` | tasks | Deduplication enforcement (task_id + model_load_id) |
| `idx_scrapes_model_created` | scrapes | Lookup scrapes for a specific model load |

## Key Design Decisions

### Why store `task_id` + `model_load_id` as unique?

`task_id` is only unique per model load session. When a model unloads and reloads, task IDs can be reused from 0 or 1. The composite key disambiguates.

### Why store preset as both raw and parsed columns?

`preset_raw` preserves the original configuration string for audit/debug purposes. Parsed columns (`ctx_size`, `threads`, etc.) enable indexed queries and sorting (e.g., "show me all tasks where `ctx_size > 8192` and `threads = 4`").

### Why store `task_ids_seen` as a comma-separated string instead of a junction table?

The scraper is a single-process writer. A junction table (`scrape_task_ids`) would add write overhead on every scrape. The string representation is compact and easily parsed for dedup checks via SQLite's `LIKE` or `instr()` functions. This avoids a tradeoff in correctness — we only ever read/parse it within the same process that writes it.

### Why store aggregate metrics in `scrapes`?

Each scrape captures a point-in-time snapshot of the server's cumulative counters. Storing them enables trend analysis (e.g., KV cache usage over time, total tokens processed per scrape window). The aggregate data is distinct from per-task data and serves different analytical questions.

### Why store boolean preset fields as `INTEGER`?

Stored as `INTEGER` (0/1) rather than `BOOLEAN` for portability and because SQLite has no native boolean type. Parsed from the preset string's `true`/`false` or `0`/`1` representations.

### Why use views instead of inline JOINs?

Views centralize the task-scrape-model joins, eliminating duplicated SQL across modules. They also serve as documentation — the view name conveys intent better than a 10-line JOIN. The code in `task.rs::get_recent` and `scrape.rs::get_recent` queries views instead of joining tables directly, making them shorter and easier to maintain.

## Example Queries

```sql
-- All completed model loads with their config
SELECT id, model_name, created, ctx_size, threads, parallel, status
FROM model_loads
WHERE status IN ('loaded', 'sleeping')
ORDER BY created DESC;

-- Average generation TPS per model load configuration (via view)
SELECT model_name, model_ctx_size, model_threads, model_parallel,
       avg_gen_tps, p95_gen_tps, task_count
FROM v_model_stats
WHERE task_count > 10
ORDER BY avg_gen_tps DESC;

-- Scatter plot data: context window vs prompt TPS (via view)
SELECT model_ctx_size, prompt_tps, n_prompt_processed, model_threads, model_parallel
FROM v_task_summary
WHERE prompt_tps > 0
ORDER BY model_ctx_size, prompt_tps;

-- Throughput vs context length (for bubble chart, via view)
SELECT context_length, gen_tps, n_gen
FROM v_task_summary
WHERE captured_at >= datetime('now', '-24 hours')
  AND context_length > 0
ORDER BY context_length;

-- Session durations for unloaded models
SELECT model_name, created,
       (unloaded_at - loaded_at) / 3600.0 AS session_hours,
       task_count
FROM v_model_history
WHERE status = 'unloaded' AND loaded_at IS NOT NULL
ORDER BY session_hours DESC;
```
