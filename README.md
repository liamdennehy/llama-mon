# llama-mon

A real-time monitoring engine for llama.cpp inference servers. Captures per-task performance metrics from a running `llama-server` instance, stores them in SQLite, and enables statistical analysis of model performance across different configurations and workloads.

Vibe-coded with [Qwen3.8-35B-A3B-MTP @ Q5_K_M](https://huggingface.co/unsloth/Qwen3.6-35B-A3B-MTP-GGUF) on an [AMD Ryzen 5 5600GT](https://www.amd.com/en/support/downloads/drivers.html/processors/ryzen/ryzen-5000-series/amd-ryzen-5-5600gt.html#amd_support_product_spec).

## Overview

`llama-mon` runs as a background daemon that periodically polls a llama.cpp server's `/models` and `/metrics` endpoints. It tracks:

- **Model loads** — each invocation of a model is recorded as a distinct entity, capturing its configuration (context size, GPU layers, threads, cache type, etc.) via the `created` timestamp and `status.preset` fields.
- **Tasks** — individual inference completions from the `/metrics?model=<name>` endpoint's `recent[]` array, including prompt processing speed, generation speed, draft token acceptance (speculative decoding), and more.
- **Scrapes** — every poll is tracked so that tasks can be deduplicated across invocations.

## Architecture

```
┌──────────────┐     /models + /metrics      ┌───────────────┐
│ llama-server │ ──────────────────────────► │  llama-mon    │
│              │                             │               │
│  - Model     │                             │  Poll Engine  │──► SQLite DB
│    Loads     │                             │  - Dedup      │
│  - Slots     │                             │  - Parse      │
│  - Tasks     │                             │  - Store      │
└──────────────┘                             └───────────────┘
```

### Components

1. **Poll Engine** — Async HTTP client that polls the llama-server at a configurable interval. Detects new model loads, tracks lifecycle transitions, and collects task metrics.

2. **SQLite Database** — Persistent store with tables for model loads, tasks, and scrapes. Deduplication ensures each task is recorded exactly once.

3. **JSON Parser** — Serde-based deserialization of the llama-server's `/models` and `/metrics` JSON responses, including extraction of preset configuration from raw key=value strings.

### Key Concepts

- **Model Load** — A single invocation of a model on the server. Identified primarily by `created_timestamp` from `/models`, with reload detection via `tasks.completed` counter (decrease = new session). Each load may have different settings, making it a distinct data point for performance analysis.
- **Task** — A completed inference request within a model load, identified by `task_id`. Contains per-token timing metrics (prompt TPS, generation TPS, draft acceptance, etc.).
- **Slot** — A processing slot allocated to a task. Up to the `parallel` setting's worth can be active simultaneously.
- **Scrape** — A single poll of the `/metrics` endpoint. Tracks which `task_id`s were seen to enable incremental deduplication.

## State Machines

- **[Model States](docs/model-states.md)** — UNLOADED → LOADING → LOADED ↔ SLEEPING → LOADED lifecycle
- **[Task/Slot States](docs/task-states.md)** — IDLE → STARTED → PROCESSING_PROMPT → DONE_PROMPT → GENERATING → IDLE

## Data Sources

- **[Models Endpoint Output](docs/models-output.md)** — Structure of `/models` JSON (model name, created timestamp, preset configuration)
- **[Metrics Endpoint Output](docs/metrics-output.md)** — Structure of `/metrics?model=<name>` JSON (task details, slot info, aggregate stats)

## Planned Features

- [x] Project structure and documentation
- [ ] SQLite schema design and migration
- [ ] HTTP polling engine with deduplication
- [ ] JSON parsing for `/models` and `/metrics` endpoints
- [ ] CLI configuration (server URL, poll interval, DB path)
- [ ] Web dashboard for statistical analysis (scatter plots: context size vs TPS, etc.)

## Dependencies (planned)

| Crate | Purpose |
|-------|---------|
| `tokio` | Async runtime |
| `reqwest` | HTTP client with JSON support |
| `serde` / `serde_json` | JSON serialization/deserialization |
| `sqlx` or `rusqlite` | SQLite database access |
| `chrono` | Timestamp handling |
| `clap` | CLI argument parsing |
| `tracing` / `tracing-subscriber` | Structured logging |
