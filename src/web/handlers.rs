//! Route handlers for the web dashboard API.
//!
//! Endpoints are broken into focused resources:
//! - `GET /api/stats/summary` — cumulative counters (lightweight)
//! - `GET /api/models` — model load inventory
//! - `GET /api/scrapes` — recent scrape snapshots (time-series points)
//! - `GET /api/tasks` — individual task records

use axum::{
    extract::{Query, State},
    http::StatusCode,
    Json,
};
use serde::Serialize;
use sqlx::SqlitePool;

use crate::db;

/// Health check endpoint.
pub async fn health() -> &'static str {
    "OK"
}

/// Status response with cumulative counters and the latest model loads.
/// Consolidates what was previously two separate calls (/stats/summary + /models).
/// Model load row as exposed to the API, augmented with scrape count.
#[derive(Debug, Serialize)]
pub struct ModelLoadApiResponse {
    pub id: i64,
    pub model_name: String,
    pub created: i64,
    pub status: String,
    pub loaded_at: Option<i64>,
    pub unloaded_at: Option<i64>,
    pub ctx_size: Option<i64>,
    pub cache_ram: Option<i64>,
    pub model_bytes: Option<i64>,
    pub scrape_count: i64,
}

impl From<(db::model_load::ModelLoadRow, i64)> for ModelLoadApiResponse {
    fn from((row, scrape_count): (db::model_load::ModelLoadRow, i64)) -> Self {
        Self {
            id: row.id,
            model_name: row.model_name,
            created: row.created,
            status: row.status,
            loaded_at: row.loaded_at,
            unloaded_at: row.unloaded_at,
            ctx_size: row.ctx_size,
            cache_ram: row.cache_ram,
            model_bytes: row.model_bytes,
            scrape_count,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct StatusResponse {
    pub summary: SummaryResponse,
    pub model_loads: Vec<ModelLoadApiResponse>,
}

// ── Shared query parameters ──────────────────────────────────

/// Common time-range filter.
#[derive(Debug, Default, serde::Deserialize)]
pub enum TimeRange {
    H1,
    H6,
    #[default]
    H24,
    D7,
}

impl TimeRange {
    pub fn hours(&self) -> u64 {
        match self {
            TimeRange::H1 => 1,
            TimeRange::H6 => 6,
            TimeRange::H24 => 24,
            TimeRange::D7 => 168,
        }
    }
}

/// Common query params for paginated endpoints.
#[derive(Debug, Default, serde::Deserialize)]
pub struct PageQuery {
    /// Max rows to return. Default: 100.
    #[serde(default = "default_limit")]
    pub limit: usize,
    /// Filter by model name. Default: all models.
    pub model: Option<String>,
    /// Time range. One of: 1h, 6h, 24h, 7d. Default: 24h.
    #[serde(default)]
    pub time_range: TimeRange,
    /// Only return rows with `scraped_at` / `captured_at` strictly after this Unix epoch second.
    /// Used by the dashboard to fetch incremental updates on each auto-refresh.
    pub since: Option<i64>,
}

fn default_limit() -> usize {
    100
}

// ── GET /api/stats/summary ───────────────────────────────────

/// Cumulative counters for the dashboard summary cards.
#[derive(Debug, Serialize)]
pub struct SummaryResponse {
    pub total_scrapes: u64,
    pub total_tasks: u64,
    pub total_model_loads: u64,
    pub total_reloads: u64,
}

/// Return cumulative counters — lightweight, no body data.
pub async fn api_summary(
    State(pool): State<SqlitePool>,
) -> Result<Json<SummaryResponse>, (StatusCode, String)> {
    let stats = db::scrape::get_global_stats(&pool).await
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to fetch global stats");
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        })?;

    Ok(Json(SummaryResponse {
        total_scrapes: stats.total_scrapes,
        total_tasks: stats.total_tasks,
        total_model_loads: stats.total_model_loads,
        total_reloads: stats.total_reloads,
    }))
}

/// GET /api/status — consolidated status with summary + latest model loads.
pub async fn api_status(
    State(pool): State<SqlitePool>,
) -> Result<Json<StatusResponse>, (StatusCode, String)> {
    let stats = db::scrape::get_global_stats(&pool).await
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to fetch global stats");
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        })?;

    let model_loads_with_counts = db::model_load::get_all_with_scrape_counts(&pool).await
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to fetch model loads");
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        })?;

    let model_loads: Vec<ModelLoadApiResponse> = model_loads_with_counts
        .into_iter()
        .map(ModelLoadApiResponse::from)
        .collect();

    Ok(Json(StatusResponse {
        summary: SummaryResponse {
            total_scrapes: stats.total_scrapes,
            total_tasks: stats.total_tasks,
            total_model_loads: stats.total_model_loads,
            total_reloads: stats.total_reloads,
        },
        model_loads,
    }))
}

// ── GET /api/models ──────────────────────────────────────────

/// Return the full model load inventory.
pub async fn api_models(
    State(pool): State<SqlitePool>,
) -> Result<Json<Vec<ModelLoadApiResponse>>, (StatusCode, String)> {
    let model_loads_with_counts = db::model_load::get_all_with_scrape_counts(&pool).await
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to fetch models");
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        })?;

    Ok(Json(model_loads_with_counts.into_iter().map(ModelLoadApiResponse::from).collect()))
}

// ── GET /api/scrapes ─────────────────────────────────────────

/// Paginated scrape snapshots for time-series charts.
pub async fn api_scrapes(
    State(pool): State<SqlitePool>,
    params: Query<PageQuery>,
) -> Result<Json<Vec<db::scrape::ScrapeRow>>, (StatusCode, String)> {
    let scrapes = db::scrape::get_recent(&pool, params.limit, params.time_range.hours(), params.model.as_deref(), params.since)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, model = ?params.model, limit = params.limit, time_range = ?params.time_range, "Failed to fetch scrapes");
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        })?;
    Ok(Json(scrapes))
}

// ── GET /api/tasks ───────────────────────────────────────────

/// Paginated individual task records.
pub async fn api_tasks(
    State(pool): State<SqlitePool>,
    params: Query<PageQuery>,
) -> Result<Json<Vec<db::task::TaskRow>>, (StatusCode, String)> {
    let tasks = db::task::get_recent(&pool, params.limit, params.time_range.hours(), params.model.as_deref(), params.since)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, model = ?params.model, limit = params.limit, time_range = ?params.time_range, "Failed to fetch tasks");
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        })?;
    Ok(Json(tasks))
}
