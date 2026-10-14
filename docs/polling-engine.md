# llama-mon Polling Engine Design

## Overview

The polling engine is an async Rust application that maintains a persistent SQLite database of llama-server metrics. It runs as a long-lived daemon, periodically querying the server and storing incremental results.

## High-Level Flow

```
┌──────────────────────────────────────────────────────────────────┐
│                        Main Loop                                 │
│                                                                  │
│  ┌─────────────┐    ┌──────────────┐    ┌────────────────────┐  │
│  │ /models      │───►│ Detect model │───►│ Upsert model_loads │  │
│  │ (every poll) │    │ loads & map  │    │ + derive preset    │  │
│  └─────────────┘    │ to DB rows   │    └────────────────────┘  │
│                     └──────┬───────┘                             │
│                            │                                     │
│                     ┌──────▼───────┐    ┌────────────────────┐  │
│                     │ For each     │───►│ /metrics?model=<n> │  │
│                     │ active load  │    │ (with dedup)       │  │
│                     └──────┬───────┘    └────────┬───────────┘  │
│                            │                     │               │
│                            │              ┌──────▼───────┐      │
│                            │              │ Parse JSON   │      │
│                            │              │ Extract      │      │
│                            │              │ tasks+agg    │      │
│                            │              └──────┬───────┘      │
│                            │                     │               │
│                            │              ┌──────▼───────┐      │
│                            │              │ Dedup against  │      │
│                            │              │ scrapes table  │      │
│                            │              └──────┬───────┘      │
│                            │                     │               │
│                            │              ┌──────▼───────┐      │
│                            │              │ INSERT tasks & │      │
│                            │              │ scrapes rows   │      │
│                            │              └──────┬───────┘      │
│                            │                     │               │
│                            │              ┌──────▼───────┐      │
│                            │              │ Update        │      │
│                            │              │ model_loads   │      │
│                            │              │ status        │      │
│                            │              └──────────────┘      │
│                            │                                     │
│                            ▼                                     │
│                    Sleep poll_interval                          │
│                    Repeat until SIGINT/SIGTERM                  │
└──────────────────────────────────────────────────────────────────┘
```

## Architecture

### Module Structure

```
src/
├── main.rs              # Entry point: CLI args, runtime setup
├── config.rs            # Configuration (server URL, poll interval, DB path)
├── poller.rs            # Main polling loop orchestrator
├── models_api.rs        # /models endpoint client + response handling
├── metrics_api.rs       # /metrics endpoint client + response handling
├── db/
│   ├── mod.rs           # Database connection, init, migrations
│   ├── schema.rs        # Schema definitions, DDL statements
│   ├── model_load.rs    # model_loads CRUD operations
│   ├── task.rs          # tasks CRUD operations
│   └── scrape.rs        # scrapes CRUD operations
├── parse/
│   ├── mod.rs           # Re-exports
│   ├── models.rs        # Deserializers for /models response
│   └── metrics.rs       # Deserializers for /metrics response
└── preset.rs            # Preset string parser (key=value → struct)
```

### Core Types

```rust
/// Represents a model load instance tracked by the scraper
struct ModelLoad {
    id: Option<i64>,                   // DB primary key (None until inserted)
    model_name: String,
    created: i64,                      // unix timestamp
    status: ModelLoadStatus,
    preset: PresetConfig,
    context_bytes: Option<i64>,
    model_bytes: Option<i64>,
    last_scraped: Option<i64>,
}

enum ModelLoadStatus {
    Unknown,
    Loaded,
    Sleeping,
    Unloaded,
}

/// A single completed task from metrics.recent[]
struct TaskRecord {
    task_id: i64,
    slot_id: i64,
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

## Polling Loop Design

### Step 1: Fetch /models

Poll the `/models` endpoint. This returns all known models on the server (active and available).

```rust
async fn fetch_models(client: &HttpClient) -> Result<ModelsResponse>
```

Parse the response into a list of known model loads:
- For each entry in `data[]`, extract `model`, `created`, `status`, and `status.preset`
- Map the status string to our enum
- Parse the preset string into structured config

### Step 2: Upsert model_loads

For each model load from Step 1:

```rust
fn upsert_model_load(conn: &mut Connection, ml: &ModelLoad) -> Result<i64>
```

- Query `model_loads` by `(model_name, created)`
- If found: update `status`, `preset_*` columns, `last_scraped`
- If not found: INSERT a new row, return the new `id`
- Return the (existing or new) `id` for use in subsequent steps

### Step 3: Detect Model Reloads via Counter Behavior

After fetching metrics, compare the `tasks.completed` counter against the previous scrape for the same model load:

```rust
fn detect_reload(
    conn: &mut Connection,
    model_load_id: i64,
    current_completed: i64,
) -> Result<Option<i64>>  // None = same session, Some(new_model_load_id) = new session
```

| Condition | Action |
|---|---|
| `current_completed > prev_completed` | Same session — no action needed |
| `current_completed == prev_completed` | No new completions — no action |
| `current_completed < prev_completed` | **Model reloaded** — close old load, start new one |
| `prev_completed == 0` and `current_completed >= 0` | First scrape or fresh load — initialize |

When a reload is detected:
1. Close the old `model_load` row (`unloaded_at = now`, `status = unloaded`)
2. Create a new `model_loads` row with the current `created` timestamp and parsed preset
3. On the first scrape of the new session, insert all `recent[]` tasks (no prior dedup exists for the new load)
4. On subsequent scrapes, resume incremental dedup via the `scrapes` table

This also catches parameter changes — if `preset_raw` differs between the old and new session, both are preserved for comparative analysis.

A model appearing in the `available[]` array with status `unloaded` is still considered "known" (not yet unloaded). Only absence from the full response (all arrays) signals unload.

### Step 4: Track Lifecycle Transitions (Secondary Signal)

The `/models` `created` timestamp provides a secondary signal:

| Transition | Action |
|---|---|
| New `(model_name, created)` pair appears in `/models` | Create new `model_loads` row (confirms reload detected via counter) |
| Model observed in `active[]` (loaded/sleeping) | Update `status`, clear `unloaded_at`, set `loaded_at` if first seen |
| Model seen in DB but absent from server response | Mark `unloaded_at = now`, `status = unloaded` |
| Model transitions LOADED → SLEEPING | Update `status` only |
| Model transitions SLEEPING → LOADED | Update `status` only |

### Step 4: Fetch /metrics for Each Active Model

For each model load currently in `loaded` or `sleeping` status:

```rust
async fn fetch_metrics(client: &HttpClient, model_name: &str) -> Result<MetricsResponse>
```

- Query `/metrics?model=<model_name>`
- Parse the full response
- Extract `active[0].metrics.recent[]` if available

### Step 5: Deduplicate and Store Tasks

```rust
fn store_tasks(conn: &mut Connection,
               model_load_id: i64,
               model_name: &str,
               model_created: i64,
               tasks: &[TaskRecord],
               aggregates: &AggregateMetrics) -> Result<usize>
```

1. Query `scrapes` for the latest `task_ids_seen` for this `(model_name, model_created)`
2. Parse the seen IDs into a `HashSet<i64>`
3. Filter incoming `tasks`: keep only those whose `task_id` is NOT in the seen set
4. Build a comma-separated string of all task IDs from this scrape (including already-seen ones)
5. INSERT new task rows (unique index on `(task_id, model_load_id)` handles any edge-case duplicates)
6. INSERT a new `scrapes` row with the aggregated metrics and the full task_id list
7. UPDATE the `model_loads.scrape_count`

### Step 6: Update Model Status

After all model loads are processed:

```rust
fn update_model_statuses(conn: &mut Connection,
                         known_loads: &[&ModelLoad]) -> Result<()>
```

- Find model_load rows whose `(model_name, created)` is NOT in `known_loads`
- Set their `status = unloaded` and `unloaded_at = now()`
- Update `status` for loads observed as `sleeping` or `loaded`

## Configuration

CLI args and/or config file:

```rust
struct Config {
    server_url: Url,           // e.g. http://localhost:8080
    poll_interval: Duration,   // e.g. 5 seconds
    db_path: PathBuf,          // e.g. ~/.local/share/llama-mon/data.db
    timeout: Duration,         // HTTP request timeout, e.g. 10 seconds
    verbose: bool,
}
```

## Error Handling

| Error | Behavior |
|---|---|
| HTTP timeout / connection refused | Log warning, sleep, retry next cycle |
| Malformed JSON response | Log warning with snippet, skip this poll |
| SQLite write error | Log error, attempt reconnect, skip this cycle |
| Model load not found in metrics | No tasks to store, continue to next model |

## Concurrency Model

Single-threaded async (tokio single-threaded runtime) is sufficient since the workload is I/O-bound with minimal CPU. The `tokio::select!` macro handles the main loop:

```rust
tokio::select! {
    _ = shutdown_recv.recv() => {
        // Graceful shutdown: drain in-flight requests, close DB
    }
    _ = sleep(interval) => {
        // Perform a poll cycle
    }
}
```

## HTTP Client Design

A wrapper around `reqwest` with:

- Configurable timeout (connect + total)
- Automatic retries (1 retry on transient errors with exponential backoff, capped at 3 retries)
- Request logging (URL, status code, duration)
- JSON response deserialization via `serde`

```rust
struct HttpClient {
    client: reqwest::Client,
    base_url: Url,
    timeout: Duration,
    max_retries: u32,
}

impl HttpClient {
    async fn get_json<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T>
    async fn get_models(&self) -> Result<ModelsResponse>
    async fn get_metrics(&self, model_name: &str) -> Result<MetricsResponse>
}
```

## Graceful Shutdown

On SIGINT or SIGTERM:
1. Set a `AtomicBool` shutdown flag
2. Stop the poll loop after the current cycle completes
3. Close all SQLite connections (ensures WAL checkpoint)
4. Exit cleanly with code 0

Logging final state:
```
[INFO]  llama-mon shutting down
[INFO]  Total model loads tracked: 3
[INFO]  Total tasks stored: 1247
[INFO]  Scrapes performed: 150
```

## Logging

Use `tracing` with `tracing-subscriber` for structured logging:

```
[2025-01-15T10:30:00Z INFO  llama_mon::poller] Poll cycle started
[2025-01-15T10:30:00Z DEBUG llama_mon::models_api] Fetched 1 model load(s)
[2025-01-15T10:30:01Z DEBUG llama_mon::metrics_api] Fetched metrics for example-model
[2025-01-15T10:30:01Z INFO  llama_mon::db::task] Inserted 2 new task(s), skipped 12 already-seen
[2025-01-15T10:30:01Z INFO  llama_mon::poller] Poll cycle completed in 1.2s
```

## Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `tokio` | 1.x | Async runtime (single-threaded) |
| `reqwest` | 0.12 | HTTP client with JSON |
| `serde` | 1.x | Serialization/deserialization |
| `serde_json` | 1.x | JSON handling |
| `sqlx` | 0.8 | SQLite (with `runtime-tokio` feature, `sqlite` feature) |
| `chrono` | 0.4 | Timestamp conversion (with `serde` feature) |
| `clap` | 4.x | CLI argument parsing |
| `tracing` | 0.1 | Structured logging |
| `tracing-subscriber` | 0.3 | Console subscriber with JSON format |
| `thiserror` | 2.x | Error type derive |
| `anyhow` | 1.x | Error context for main |
| `signal-hook-tokio` | 0.3 | Signal handling |
