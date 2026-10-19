# Frontend Design — Embedded Web Dashboard

## Status: ✅ Implemented (v0.2.0)

All planned features are implemented and tested. See the implementation tracker at `docs/implementation-tracker.md`.

## Overview

llama-mon includes an optional embedded HTTP server that serves a single-page
dashboard for visualizing inference performance data persisted in SQLite.

The server is **enabled by default** on port 8081. It can be disabled with
`--no-web`. The scraper (poller) is also **enabled by default** and can be
disabled with `--no-scraper`. This allows three modes:

| Mode | Flags | Description |
|------|-------|-------------|
| Full daemon (default) | *(none)* | Polls + persists + serves web dashboard |
| Scraper only | `--no-web` | Polls + persists, no web server |
| Reporter only | `--no-scraper` | No polling, serves web dashboard |

The scraper-only and reporter-only modes enable a distributed deployment
where a dedicated scraper process writes to a shared database, and one or
more reporter processes serve the dashboard from the same DB file.

## Architecture

```
┌─────────────────────────────────────────────┐
│                  llama-mon                   │
│                                              │
│  ┌──────────┐    ┌───────────┐              │
│  │ Poller   │───▶│  SQLite   │              │
│  │ (tokio)  │    │  (WAL)    │              │
│  └──────────┘    └─────┬─────┘              │
│                        │                     │
│  ┌──────────┐    ┌─────▼─────┐              │
│  │   axum   │◀───│  /api/    │              │
│  │  server  │    │  routes   │              │
│  └────┬─────┘    └───────────┘              │
│       │                                     │
│  ┌────▼─────┐                               │
│  │ index    │                               │
│  │ .html    │                               │
│  └──────────┘                               │
└─────────────────────────────────────────────┘
```

- **Single Rust process** — one daemon, one DB pool, one HTTP listener
- **No reverse proxy needed** — static files and API share the same server
- **Lightweight** — `axum` + `tokio` runtime already in the dependency tree
- **Internal use only** — no auth required (operator-access machine only)

## Module Structure

```
src/
├── web/
│   ├── mod.rs          # serve() entry point, router setup, shutdown handling
│   ├── handlers.rs     # Route handlers: summary, models, scrapes, tasks
│   └── static_files.rs # Embedded index.html, MIME type helpers
├── config.rs           # --web-port, --no-web, --no-scraper flags
├── main.rs             # Conditional poller/web spawning, shared shutdown channel
└── poller.rs           # run_with_shutdown() for broadcast-based shutdown
```

## Configuration

New `Config` fields:

```rust
pub struct Config {
    // ... existing fields ...
    pub web_port: u16,          // Dashboard port (default 8081)
    pub no_web: bool,           // Skip web server (--no-web)
    pub no_scraper: bool,       // Skip poller (--no-scraper)
}
```

CLI flags: `--web-port PORT`, `--no-web`, `--no-scraper` (env: `LLAMA_MON_WEB_PORT`)

## API Endpoints

### `GET /`

Serves the single-page dashboard (index.html, embedded via `include_bytes!`).

### `GET /api/stats/summary`

Returns cumulative counters only. Lightweight, no body data.

```json
{
  "total_scrapes": 42,
  "total_tasks": 156,
  "total_model_loads": 3,
  "total_reloads": 1
}
```

### `GET /api/models`

Returns the full model load inventory.

```json
[
  {
    "id": 1,
    "model_name": "example-model",
    "created": 1792183436,
    "status": "loaded",
    "scrape_count": 42,
    "ctx_size": 131072,
    "unloaded_at": null,
    ...
  }
]
```

### `GET /api/scrapes`

Recent scrape snapshots for time-series charts.

| Param | Type | Default | Description |
|-------|------|---------|-------------|
| `limit` | int | 100 | Max rows |
| `model` | string | — | Filter by model name |
| `time_range` | string | 24h | One of: 1h, 6h, 24h, 7d |

Response: `ScrapeRow[]` (same schema as DB row: id, model_name, scraped_at, tasks_completed, tasks_duration_s, context_max, prompt/prediction/decode totals, memory bytes, etc.)

### `GET /api/tasks`

Individual task records. Same query params as `/api/scrapes`.

Response: `TaskRow[]` (task_id, model_name, slot_id, n_gen, gen_tps, gen_per_token_ms, prompt metrics, draft metrics, captured_at)

### `GET /health`

Returns `200 OK` with body `"OK"` for health checks.

## Frontend Features

### Tech Stack

| Component | Choice | Rationale |
|-----------|--------|-----------|
| CSS | Custom (CSS variables) | No build step, dark/light theming via `data-theme` |
| JS | Vanilla | No build step, fetch API, `Promise.all` |
| Charts | Chart.js 4 (CDN) | Time-series, lightweight, no build step |
| Icons | Unicode | No heavy icon library |
| Layout | CSS Grid + Flexbox | Responsive, no framework |

### Implemented Features

| Feature | Status | Details |
|---------|--------|---------|
| Summary cards | ✅ | Total scrapes, tasks, model loads, reloads |
| Model table | ✅ | Name, status badge, created, scrape count, context size |
| Time-series charts | ✅ | Tasks completed (line), Gen TPS (scatter), Gen latency (scatter), Memory (line) |
| Throughput vs Context | ✅ | Bubble chart: x=context length, y=Gen/Prompt TPS, radius=sqrt(tokens generated), metric toggle |
| Recent tasks table | ✅ | Last 50 rows with count indicator |
| Model filter dropdown | ✅ | Dynamically populated from `/api/models` |
| Time-range selector | ✅ | 1h, 6h, 24h, 7d |
| Auto-refresh toggle | ✅ | 30s interval, toggle button (●/○ indicator) |
| Manual refresh | ✅ | ↻ Refresh button |
| CSV export | ✅ | Exports scrapes with ISO timestamps |
| Dark/light theme | ✅ | Persisted in localStorage, toggles chart grid colors |
| Error banner | ✅ | Non-intrusive, shows API error messages |
| Print stylesheet | ✅ | Hides controls/charts, shows tables |
| Responsive layout | ✅ | CSS Grid with `auto-fit`/`minmax` |
| Loading states | ✅ | Spinner overlay during initial load |

### Refresh Strategy

- **Auto-polling**: `fetch()` to all 4 endpoints in parallel every 30 seconds
- **Manual refresh**: Click the ↻ Refresh button
- **Auto-refresh toggle**: ● = enabled, ○ = disabled (for detailed analysis)
- **No WebSocket/SSE**: Page load always shows latest DB state

### Error Handling

| Scenario | Behavior |
|----------|----------|
| API connection failed | Red error banner, retry on next auto-refresh cycle |
| No data yet | Cards show "—", charts render empty |
| Web port in use | Daemon exits with: "Failed to bind web port N" |
| Scraper disabled | Dashboard loads but shows stale/zero data |

### Design Principles

- **Dark theme by default** — easier on the eyes for long-running monitoring
- **No login/auth** — operator runs on internal machine, no external exposure
- **Print-friendly** — `@media print` stylesheet for generating reports
- **Responsive** — works on laptop and tablet
- **Self-contained** — single HTML file, zero build step

## Security

- **No authentication** — operator-access machine only
- **No CORS** — same-origin only, not exposed externally
- **Rate limiting** — not needed (single operator, 30s refresh)
- **HTTPS** — not required (internal access only)
