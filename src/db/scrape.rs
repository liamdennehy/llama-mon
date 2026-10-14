//! scrapes table CRUD operations and CSV helper functions.

use sqlx::SqlitePool;

use crate::db::DbError;

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

    #[tokio::test]
    #[ignore = "requires database"]
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

    async fn test_pool() -> SqlitePool {
        let pool = SqlitePool::connect(":memory:").await.unwrap();
        crate::db::schema::init_db(&pool).await.unwrap();
        pool
    }
}
