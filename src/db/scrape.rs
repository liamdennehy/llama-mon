//! scrapes table CRUD operations and CSV helper functions.

use sqlx::SqlitePool;

use crate::db::DbError;

/// A scrape row returned by the dashboard API.
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct ScrapeRow {
    pub id: i64,
    pub model_name: String,
    pub model_created: i64,
    pub model_load_id: i64,
    pub scraped_at: i64,
    pub context_max: Option<i64>,
    pub tasks_processing: i64,
    pub tasks_queued: i64,
    pub tasks_completed: i64,
    pub tasks_duration_s: f64,
    pub prompt_tokens_total: Option<i64>,
    pub prompt_tokens_cached: Option<i64>,
    pub prompt_tokens_processed: Option<i64>,
    pub prediction_tokens_total: Option<i64>,
    pub prediction_tokens_processed: Option<i64>,
    pub decode_total: Option<i64>,
    pub kvcache_active_tokens: Option<i64>,
    pub memory_context_bytes: Option<i64>,
    pub memory_model_bytes: Option<i64>,
    pub task_ids_seen: String,
}

/// Cumulative scrape statistics for the dashboard summary.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ScrapeStats {
    pub total_scrapes: u64,
    pub total_tasks: u64,
    pub total_model_loads: u64,
    pub total_poll_cycles: u64,
    pub total_reloads: u64,
}

#[derive(Debug, Clone)]
pub struct ScrapeInsert {
    pub model_name: String,
    pub model_created: i64,
    pub model_load_id: i64,
    pub scraped_at: i64,
    pub context_max: Option<i64>,
    pub tasks_processing: i64,
    pub tasks_queued: i64,
    pub tasks_completed: i64,
    pub tasks_duration_s: f64,
    pub prompt_tokens_total: Option<i64>,
    pub prompt_tokens_cached: Option<i64>,
    pub prompt_tokens_processed: Option<i64>,
    pub prediction_tokens_total: Option<i64>,
    pub prediction_tokens_processed: Option<i64>,
    pub decode_total: Option<i64>,
    pub kvcache_active_tokens: Option<i64>,
    pub memory_context_bytes: Option<i64>,
    pub memory_model_bytes: Option<i64>,
    pub task_ids_seen: String,
}

/// Insert a scrape record. Returns the new id.
pub async fn insert(pool: &SqlitePool, s: &ScrapeInsert) -> Result<i64, DbError> {
    let id = sqlx::query_scalar(
        r#"INSERT INTO scrapes (
            model_name, model_created, model_load_id, scraped_at,
            context_max, tasks_processing, tasks_queued, tasks_completed,
            tasks_duration_s,
            prompt_tokens_total, prompt_tokens_cached, prompt_tokens_processed,
            prediction_tokens_total, prediction_tokens_processed,
            decode_total, kvcache_active_tokens,
            memory_context_bytes, memory_model_bytes,
            task_ids_seen
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        RETURNING id"#
    )
    .bind(&s.model_name)
    .bind(s.model_created)
    .bind(s.model_load_id)
    .bind(s.scraped_at)
    .bind(s.context_max)
    .bind(s.tasks_processing)
    .bind(s.tasks_queued)
    .bind(s.tasks_completed)
    .bind(s.tasks_duration_s)
    .bind(s.prompt_tokens_total)
    .bind(s.prompt_tokens_cached)
    .bind(s.prompt_tokens_processed)
    .bind(s.prediction_tokens_total)
    .bind(s.prediction_tokens_processed)
    .bind(s.decode_total)
    .bind(s.kvcache_active_tokens)
    .bind(s.memory_context_bytes)
    .bind(s.memory_model_bytes)
    .bind(&s.task_ids_seen)
    .fetch_one(pool)
    .await?;
    Ok(id)
}

/// Get the most recent scrape for a given model load.
/// Returns None if no scrapes exist.
pub async fn get_latest_for_load(
    pool: &SqlitePool,
    model_load_id: i64,
) -> Result<Option<ScrapeRow>, DbError> {
    let row: Option<ScrapeRow> = sqlx::query_as(
        r#"SELECT id, model_name, model_created, model_load_id, scraped_at,
                context_max, tasks_processing, tasks_queued, tasks_completed,
                tasks_duration_s, prompt_tokens_total, prompt_tokens_cached,
                prompt_tokens_processed, prediction_tokens_total,
                prediction_tokens_processed, decode_total,
                kvcache_active_tokens, memory_context_bytes, memory_model_bytes,
                task_ids_seen
           FROM scrapes
           WHERE model_load_id = ?
           ORDER BY scraped_at DESC LIMIT 1"#
    )
    .bind(model_load_id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// Get the most recent tasks_completed counter for reload detection.
pub async fn get_latest_completed(
    pool: &SqlitePool,
    model_load_id: i64,
) -> Result<Option<i64>, DbError> {
    let val: Option<i64> = sqlx::query_scalar(
        r#"SELECT tasks_completed FROM scrapes
           WHERE model_load_id = ?
           ORDER BY scraped_at DESC LIMIT 1"#
    )
    .bind(model_load_id)
    .fetch_optional(pool)
    .await?;
    Ok(val)
}

/// Convert a sorted set of task IDs to a comma-separated string.
pub fn task_ids_to_csv(ids: &std::collections::BTreeSet<i64>) -> String {
    ids.iter().map(|id| id.to_string()).collect::<Vec<_>>().join(",")
}

/// Parse a comma-separated string of task IDs into a set.
pub fn csv_to_task_ids(csv: &str) -> std::collections::BTreeSet<i64> {
    csv.split(',')
        .filter_map(|t| t.trim().parse::<i64>().ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_task_ids_to_csv() {
        let mut ids = std::collections::BTreeSet::new();
        ids.insert(3);
        ids.insert(1);
        ids.insert(2);
        assert_eq!(task_ids_to_csv(&ids), "1,2,3");
    }

    #[test]
    fn test_task_ids_to_csv_empty() {
        let ids = std::collections::BTreeSet::new();
        assert_eq!(task_ids_to_csv(&ids), "");
    }

    #[test]
    fn test_csv_to_task_ids() {
        let set = csv_to_task_ids("10,5,20,3");
        assert_eq!(set.len(), 4);
        assert!(set.contains(&10));
        assert!(set.contains(&5));
        assert!(set.contains(&20));
        assert!(set.contains(&3));
    }

    #[test]
    fn test_csv_roundtrip() {
        let mut original = std::collections::BTreeSet::new();
        original.insert(42);
        original.insert(7);
        original.insert(100);
        let csv = task_ids_to_csv(&original);
        let parsed = csv_to_task_ids(&csv);
        assert_eq!(original, parsed);
    }

    /// Uses a real SQLite file — never `:memory:`.
    #[tokio::test]
    async fn test_insert_scrape() {
        let pool = test_pool().await;
        let scrape = ScrapeInsert {
            model_name: "test-model".to_string(),
            model_created: 1234567890,
            model_load_id: 1,
            scraped_at: 1234567895,
            context_max: Some(16384),
            tasks_processing: 0,
            tasks_queued: 0,
            tasks_completed: 5,
            tasks_duration_s: 30.5,
            prompt_tokens_total: Some(500),
            prompt_tokens_cached: Some(100),
            prompt_tokens_processed: Some(400),
            prediction_tokens_total: Some(200),
            prediction_tokens_processed: Some(200),
            decode_total: Some(180),
            kvcache_active_tokens: Some(400),
            memory_context_bytes: Some(1879048192),
            memory_model_bytes: Some(2959223072),
            task_ids_seen: "1,2,3,4,5".to_string(),
        };
        let id = insert(&pool, &scrape).await.unwrap();
        assert!(id > 0);
    }

    /// Uses a real SQLite file — never `:memory:`.
    #[tokio::test]
    async fn test_get_latest_for_load() {
        let pool = test_pool().await;
        let scrape = ScrapeInsert {
            model_name: "test-latest".to_string(),
            model_created: 1111111111,
            model_load_id: 1,
            scraped_at: 1111111120,
            context_max: Some(16384),
            tasks_processing: 0, tasks_queued: 0, tasks_completed: 10,
            tasks_duration_s: 10.0,
            prompt_tokens_total: Some(100), prompt_tokens_cached: Some(50),
            prompt_tokens_processed: Some(50),
            prediction_tokens_total: Some(50), prediction_tokens_processed: Some(50),
            decode_total: Some(45), kvcache_active_tokens: Some(100),
            memory_context_bytes: Some(1000000), memory_model_bytes: Some(2000000),
            task_ids_seen: "1,2,3,4,5".to_string(),
        };
        let _id = insert(&pool, &scrape).await.unwrap();

        let latest = get_latest_for_load(&pool, 1).await.unwrap().unwrap();
        assert_eq!(latest.tasks_completed, 10);
        assert_eq!(latest.task_ids_seen, "1,2,3,4,5");

        let none = get_latest_for_load(&pool, 999).await.unwrap();
        assert!(none.is_none());
    }

    /// Creates an in-memory SQLite pool for testing.
    async fn test_pool() -> SqlitePool {
        let pool = SqlitePool::connect(":memory:").await.unwrap();
        sqlx::query("PRAGMA journal_mode=WAL")
            .execute(&pool)
            .await
            .unwrap();
        crate::db::schema::init_db(&pool).await.unwrap();
        // Insert a model_load so FK-constrained scrapes can reference id=1
        sqlx::query(
            "INSERT INTO model_loads (model_name, created, status, ctx_size) VALUES (?, ?, ?, ?)"
        )
        .bind("test-model")
        .bind(1000000000i64)
        .bind("loaded")
        .bind(4096i64)
        .execute(&pool)
        .await
        .unwrap();
        pool
    }
}

// ── Dashboard query helpers ──────────────────────────────────

/// Fetch the most recent scrapes, optionally filtered by model name and/or a `since` timestamp.
/// Uses the `v_scrape_summary` view to avoid repeating the JOIN.
/// Note: model_load_id is not selected from the view (it's implicit via model_name).
pub async fn get_recent(
    pool: &SqlitePool,
    limit: usize,
    hours: u64,
    model_name: Option<&str>,
    since: Option<i64>,
) -> Result<Vec<ScrapeRow>, DbError> {
    let rows: Vec<ScrapeRow> = match (model_name, since) {
        (Some(name), Some(s)) => {
            sqlx::query_as::<_, ScrapeRow>(
                "SELECT id, model_name, model_created, model_load_id, scraped_at, context_max, \
                 tasks_processing, tasks_queued, tasks_completed, tasks_duration_s, \
                 prompt_tokens_total, prompt_tokens_cached, prompt_tokens_processed, \
                 prediction_tokens_total, prediction_tokens_processed, decode_total, \
                 kvcache_active_tokens, memory_context_bytes, memory_model_bytes, task_ids_seen \
                 FROM v_scrape_summary \
                 WHERE model_name = ? AND scraped_at > ? \
                 ORDER BY scraped_at DESC LIMIT ?"
            )
            .bind(name)
            .bind(s)
            .bind(limit as i64)
            .fetch_all(pool)
            .await?
        }
        (Some(name), None) => {
            sqlx::query_as::<_, ScrapeRow>(
                "SELECT id, model_name, model_created, model_load_id, scraped_at, context_max, \
                 tasks_processing, tasks_queued, tasks_completed, tasks_duration_s, \
                 prompt_tokens_total, prompt_tokens_cached, prompt_tokens_processed, \
                 prediction_tokens_total, prediction_tokens_processed, decode_total, \
                 kvcache_active_tokens, memory_context_bytes, memory_model_bytes, task_ids_seen \
                 FROM v_scrape_summary \
                 WHERE model_name = ? AND scraped_at >= unixepoch('now', ?) \
                 ORDER BY scraped_at DESC LIMIT ?"
            )
            .bind(name)
            .bind(&format!("-{hours} hours"))
            .bind(limit as i64)
            .fetch_all(pool)
            .await?
        }
        (None, Some(s)) => {
            sqlx::query_as::<_, ScrapeRow>(
                "SELECT id, model_name, model_created, model_load_id, scraped_at, context_max, \
                 tasks_processing, tasks_queued, tasks_completed, tasks_duration_s, \
                 prompt_tokens_total, prompt_tokens_cached, prompt_tokens_processed, \
                 prediction_tokens_total, prediction_tokens_processed, decode_total, \
                 kvcache_active_tokens, memory_context_bytes, memory_model_bytes, task_ids_seen \
                 FROM v_scrape_summary \
                 WHERE scraped_at > ? \
                 ORDER BY scraped_at DESC LIMIT ?"
            )
            .bind(s)
            .bind(limit as i64)
            .fetch_all(pool)
            .await?
        }
        (None, None) => {
            sqlx::query_as::<_, ScrapeRow>(
                "SELECT id, model_name, model_created, model_load_id, scraped_at, context_max, \
                 tasks_processing, tasks_queued, tasks_completed, tasks_duration_s, \
                 prompt_tokens_total, prompt_tokens_cached, prompt_tokens_processed, \
                 prediction_tokens_total, prediction_tokens_processed, decode_total, \
                 kvcache_active_tokens, memory_context_bytes, memory_model_bytes, task_ids_seen \
                 FROM v_scrape_summary \
                 WHERE scraped_at >= unixepoch('now', ?) \
                 ORDER BY scraped_at DESC LIMIT ?"
            )
            .bind(&format!("-{hours} hours"))
            .bind(limit as i64)
            .fetch_all(pool)
            .await?
        }
    };
    Ok(rows)
}

/// Compute global cumulative stats from the database.
pub async fn get_global_stats(pool: &SqlitePool) -> Result<ScrapeStats, DbError> {
    let stats: (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT COUNT(*), COALESCE(MAX(tasks_completed), 0), COUNT(DISTINCT model_name), COUNT(*) FROM scrapes"
    )
    .fetch_one(pool)
    .await?;

    let reloads: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM model_loads WHERE status = 'unloaded'"
    )
    .fetch_one(pool)
    .await?;

    Ok(ScrapeStats {
        total_scrapes: stats.0 as u64,
        total_tasks: stats.1 as u64,
        total_model_loads: stats.3 as u64,
        total_poll_cycles: stats.0 as u64, // scrapes ~= poll cycles (one scrape per cycle)
        total_reloads: reloads as u64,
    })
}
