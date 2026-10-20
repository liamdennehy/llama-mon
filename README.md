# llama-mon

A real-time monitoring engine for llama.cpp inference servers. Captures per-task performance metrics from a running `llama-server` instance, stores them in SQLite, and provides a web dashboard for visual analysis of model performance.

Vibe-coded with [Qwen3.8-35B-A3B-MTP @ Q5_K_M](https://huggingface.co/unsloth/Qwen3.6-35B-A3B-MTP-GGUF) on an [AMD Ryzen 5 5600GT](https://www.amd.com/en/support/downloads/drivers.html/processors/ryzen/ryzen-5000-series/amd-ryzen-5-5600gt.html#amd_support_product_spec).

## Overview

`llama-mon` runs as a background daemon that periodically polls a llama.cpp server's `/models` and `/metrics` endpoints. It tracks:

- **Model loads** — each invocation of a model is recorded as a distinct entity, capturing its configuration (context size, GPU layers, threads, cache type, etc.) via the `created` timestamp and `status.preset` fields.
- **Tasks** — individual inference completions from the `/metrics?model=<name>` endpoint's `recent[]` array, including prompt processing speed, generation speed, draft token acceptance (speculative decoding), KV cache hit rates, and more.
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
                                                     │
                                                     ▼
                                              ┌───────────────┐
                                              │  Web Dashboard│
                                              │  :8215 (default) │
                                              └───────────────┘
```

### Components

1. **Poll Engine** — Async HTTP client that polls the llama-server at a configurable interval. Detects new model loads, tracks lifecycle transitions, and collects task metrics.

2. **SQLite Database** — Persistent store with tables for model loads, tasks, and scrapes. Deduplication ensures each task is recorded exactly once.

3. **JSON Parser** — Serde-based deserialization of the llama-server's `/models` and `/metrics` JSON responses, including extraction of preset configuration from raw key=value strings.

### Key Concepts

- **Model Load** — A single invocation of a model on the server. Identified primarily by `created_timestamp` from `/models`, with reload detection via `tasks.completed` counter (decrease = new session). Each load may have different settings, making it a distinct data point for performance analysis.
- **Task** — A completed inference request within a model load, identified by `task_id`. Contains per-token timing metrics (prompt TPS, generation TPS, draft acceptance, KV cache hit rate, etc.).
- **Slot** — A processing slot allocated to a task. Up to the `parallel` setting's worth can be active simultaneously.
- **Scrape** — A single poll of the `/metrics` endpoint. Tracks which `task_id`s were seen to enable incremental deduplication.

## Web Dashboard

The web dashboard (enabled by default on `:8215`) provides real-time monitoring of your llama.cpp server:

- **Summary cards** — Tasks completed, current tasks processing/queued, average prompt/generation TPS.
- **Throughput vs Context Length** — Bubble chart plotting context length against TPS (selectable: generation or prompt). Bubble size represents token count. Click points to see detailed tooltip information.
- **Recent Tasks** — Table of the latest inference tasks, showing generation and prompt throughput, KV cache hit rate, and timing per token. The number of displayed rows can be adjusted via a dropdown (10, 50, 200, 1000).
- **Recent Scrapes** — Table of poll results, showing aggregate statistics per scrape, with CSV export.
- **Models** — Table of loaded models with status, context size, MTP draft stats, and VRAM usage.
- **Model timeline** — Scatter plot of generation TPS over time for each model load.

### Dashboard features

- **Time range** — Filter data by 1h, 6h, 24h, or 7d. Data limits scale to the range (no hard-capped row limits).
- **Model filter** — Show data for a specific model or all models.
- **Theme toggle** — Dark and light modes, persisted in `localStorage`.
- **Auto-refresh** — Updates every 30 seconds; can be toggled off.
- **CSV export** — Download scrape data for offline analysis.
- **Incremental updates** — After initial load, only new data is fetched to minimize server load.

### Web server modes

| Mode | `--no-web` | `--no-scraper` | Description |
|------|-----------|----------------|-------------|
| Full daemon | off | off | Polls + persists + serves dashboard |
| Scraper only | on | off | Polls + persists, no web server |
| Reporter only | off | on | No polling, serves dashboard |

## Command Line

```bash
llama-mon [OPTIONS]
```

| Option | Description | Default |
|--------|-------------|---------|
| `--server, -s` | llama.cpp server URL(s) to monitor (repeatable) | Required |
| `--port, -p` | Web dashboard port | `8215` |
| `--no-port` | Disable web dashboard | off |
| `--no-web` | Disable web dashboard (alias for `--no-port`) | — |
| `--no-scraper` | Disable polling (reporter-only mode) | off |
| `--interval, -i` | Poll interval (e.g. `30s`, `5m`) | `30s` |
| `--timeout, -t` | HTTP timeout | `10s` |
| `--models, -m` | Filter by model name(s) (repeatable) | All |
| `--db, -d` | SQLite database path | `llama-mon.db` |
| `--listen, -l` | Web dashboard listen address | `0.0.0.0` |
| `--verbose, -v` | Increase log verbosity | quiet |
| `--silent` | Suppress all log output | off |
| `--fail-fast` | Exit on first server unreachable | off |

Configuration can also be provided via a `.env` file (auto-discovered in the current directory) or environment variables (`LLAMA_MON_SERVER`, `LLAMA_MON_PORT`, etc.).

### Logging

- `--verbose` / `-v` — Sets log level to `info`. Add `-vv` for `debug`, `-vvv` for `trace`.
- `RUST_LOG` environment variable takes precedence over CLI flags.
- `--silent` — No log output at all (startup banner still prints to stdout).

## Configuration

A sample `.env` file is provided in `sample.env`. Key variables:

| Variable | Description | Default |
|----------|-------------|--------|
| `LLAMA_MON_SERVER` | Comma-separated list of llama-server URLs | — |
| `LLAMA_MON_PORT` | Web dashboard port | `8215` |
| `LLAMA_MON_DB` | SQLite database path | `llama-mon.db` |
| `LLAMA_MON_INTERVAL` | Poll interval | `30s` |
| `LLAMA_MON_TIMEOUT` | HTTP timeout | `10s` |
| `LLAMA_MON_MODEL_FILTER` | Comma-separated model names to filter | (none) |
| `LLAMA_MON_FAIL_FAST` | Exit on first server unreachable | `false` |
| `LLAMA_MON_NO_WEB` | Disable web dashboard | `false` |
| `LLAMA_MON_NO_SCRAPER` | Disable polling | `false` |

## Data Sources

- **[Models Endpoint Output](docs/models-output.md)** — Structure of `/models` JSON (model name, created timestamp, preset configuration)
- **[Metrics Endpoint Output](docs/metrics-output.md)** — Structure of `/metrics?model=<name>` JSON (task details, slot info, aggregate stats)

## Planned Features

- [x] Project structure and documentation
- [x] SQLite schema design and migration
- [x] HTTP polling engine with deduplication
- [x] JSON parsing for `/models` and `/metrics` endpoints
- [x] CLI configuration (server URL, poll interval, DB path)
- [x] Web dashboard for real-time monitoring and statistical analysis
- [ ] Export data to Prometheus / Grafana
- [ ] Alerting on TPS thresholds
- [ ] Multi-server aggregation view
- [ ] Task-level deduplication across scrapes (currently per-scope)

## State Machines

- **[Model States](docs/model-states.md)** — UNLOADED → LOADING → LOADED ↔ SLEEPING → LOADED lifecycle
- **[Task/Slot States](docs/task-states.md)** — IDLE → STARTED → PROCESSING_PROMPT → DONE_PROMPT → GENERATING → IDLE
