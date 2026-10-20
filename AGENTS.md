# AGENTS.md

Instructions for AI assistants working on llama-mon.

## Project Context

llama-mon is a Rust 2024 async daemon that polls a llama.cpp inference server's `/models` and `/metrics` endpoints and persists task-level performance data to SQLite. Read `docs/` for full specifications before implementing.

## Style

- Use `tracing` for all logging; never `println!` outside of startup/shutdown banners
- Prefer `thiserror` for library error types, `anyhow` only at the `main()` boundary
- Struct fields: snake_case; HTTP/API fields keep their original casing
- All doc comments use `///`; internal modules get a `//` block comment at the top explaining the module's role
- `TODO:` markers are for human attention; `HACK:` for known deviations from the spec

## Commit Rules

- Every commit must include: `Assisted-by: "Spark AI" <spark@wiemax.net>`
- Never claim authorship; use `Assisted-by:` only, never `Co-authored-by:`
- Write meaningful commit messages; avoid "fix" or "update" without context

## Database

- All DDL is in `db/schema.rs`; do not write raw SQL strings outside that file
- Use prepared statements with bind parameters; never interpolate values into SQL
- WAL mode is required; checkpoint on shutdown
- Prefer SQL Views over lengthy SELECT statements in the source code

## HTTP

- All requests go through `HttpClient` in `http_client.rs` — do not call `reqwest` directly
- Timeout applies to the entire request; never use individual connect or read timeouts
- Retry only on transient errors (connection refused, timeout, 5xx); do not retry 4xx

## Parsing

- All serde deserialization lives in `parse/`; do not deserialize inline in the poller
- Missing fields use `#[serde(default)]` — never unwrap a missing JSON key
- Preset parsing ignores unknown keys silently

## Testing

- Unit tests go in the same file as the code (`#[cfg(test)] mod tests`)
- Test preset parsing against the sample in `docs/models-output.md`
- Test metrics parsing against the sample in `docs/metrics-output.md`
- Integration tests in `tests/integration.rs` target a live llama-server (default: `http://10.100.0.1:8214`)
- Integration tests use `skip_unreachable` helper — they pass on CI without a live server

## Web Dashboard

- Dashboard is a single file: `web/index.html` — no build step, no framework
- Static files are embedded in the binary via `include_bytes!` in `web/static_files.rs`
- API endpoints are split: `/api/stats/summary`, `/api/models`, `/api/scrapes`, `/api/tasks`
- All 3 data endpoints share query params: `limit`, `model`, `time_range`
- Frontend fetches all endpoints in parallel via `Promise.all`
- Theme (dark/light) persisted in `localStorage` as `llama-mon-theme`
- Auto-refresh interval: 30 seconds (toggleable)
- CSV export: scrapes only, ISO timestamps

## Web Server Modes

llama-mon supports three operational modes:

| Mode | `--no-web` | `--no-scraper` | Description |
|------|-----------|----------------|-------------|
| Full daemon | off | off | Polls + persists + serves dashboard |
| Scraper only | on | off | Polls + persists, no web server |
| Reporter only | off | on | No polling, serves dashboard |

Shutdown uses a shared `tokio::sync::broadcast::channel(1)` between poller and web server.
