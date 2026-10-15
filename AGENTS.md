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
- No integration tests yet — that's a Phase 5 concern
