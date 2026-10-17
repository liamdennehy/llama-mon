//! tasks table CRUD operations.

use sqlx::{SqlitePool, QueryBuilder};
use std::collections::HashSet;

use crate::db::DbError;

#[derive(Debug, Clone)]
pub struct TaskInsert {
    pub task_id: i64,
    pub model_load_id: i64,
    pub slot_id: i64,
    pub n_prompt_cached: i64,
    pub n_prompt_processed: i64,
    pub prompt_ms: f64,
    pub prompt_tps: f64,
    pub prompt_per_token_ms: f64,
    pub n_gen: i64,
    pub gen_ms: f64,
    pub gen_tps: f64,
    pub gen_per_token_ms: f64,
    pub n_draft_tokens: i64,
    pub n_draft_accepted: i64,
    pub captured_at: i64,
}

/// Bulk insert tasks. Returns the number of rows inserted.
/// Skips duplicates silently via INSERT OR IGNORE.
pub async fn bulk_insert(
    pool: &SqlitePool,
    tasks: &[TaskInsert],
) -> Result<usize, DbError> {
    if tasks.is_empty() {
        return Ok(0);
    }

    let mut qb = QueryBuilder::new(
        "INSERT OR IGNORE INTO tasks (
            task_id, model_load_id, slot_id,
            n_prompt_cached, n_prompt_processed, prompt_ms, prompt_tps, prompt_per_token_ms,
            n_gen, gen_ms, gen_tps, gen_per_token_ms,
            n_draft_tokens, n_draft_accepted, captured_at
        ) "
    );

    qb.push_values(tasks, |mut b, task| {
        b.push_bind(task.task_id)
         .push_bind(task.model_load_id)
         .push_bind(task.slot_id)
         .push_bind(task.n_prompt_cached)
         .push_bind(task.n_prompt_processed)
         .push_bind(task.prompt_ms)
         .push_bind(task.prompt_tps)
         .push_bind(task.prompt_per_token_ms)
         .push_bind(task.n_gen)
         .push_bind(task.gen_ms)
         .push_bind(task.gen_tps)
         .push_bind(task.gen_per_token_ms)
         .push_bind(task.n_draft_tokens)
         .push_bind(task.n_draft_accepted)
         .push_bind(task.captured_at);
    });

    let result = qb.build().execute(pool).await?;
    Ok(result.rows_affected() as usize)
}

/// Get the set of task_ids seen in the most recent scrape for this model load.
pub async fn get_seen_task_ids(
    pool: &SqlitePool,
    model_name: &str,
    model_created: i64,
) -> Result<HashSet<i64>, DbError> {
    let csv: Option<String> = sqlx::query_scalar::<_, String>(
        r#"SELECT task_ids_seen FROM scrapes
           WHERE model_name = ? AND model_created = ?
           ORDER BY scraped_at DESC LIMIT 1"#
    )
    .bind(model_name)
    .bind(model_created)
    .fetch_optional(pool)
    .await?;

    Ok(csv
        .map(|s| s.split(',')
            .filter_map(|t| t.trim().parse::<i64>().ok())
            .collect())
        .unwrap_or_default())
}

/// Get the last scraped tasks_completed counter for reload detection.
pub async fn get_last_completed_counter(
    pool: &SqlitePool,
    model_name: &str,
    model_created: i64,
) -> Result<Option<i64>, DbError> {
    let val: Option<i64> = sqlx::query_scalar::<_, i64>(
        r#"SELECT tasks_completed FROM scrapes
           WHERE model_name = ? AND model_created = ?
           ORDER BY scraped_at DESC LIMIT 1"#
    )
    .bind(model_name)
    .bind(model_created)
    .fetch_optional(pool)
    .await?;
    Ok(val)
}

/// Get all task_ids ever stored for a model load (fallback dedup).
pub async fn get_all_task_ids_for_load(
    pool: &SqlitePool,
    model_load_id: i64,
) -> Result<HashSet<i64>, DbError> {
    let ids: Vec<i64> = sqlx::query_scalar(
        "SELECT task_id FROM tasks WHERE model_load_id = ?"
    )
    .bind(model_load_id)
    .fetch_all(pool)
    .await?;
    Ok(ids.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires database"]
    async fn test_bulk_insert() {
        let pool = test_pool().await;
        let tasks = vec![
            TaskInsert {
                task_id: 1, model_load_id: 1, slot_id: 0,
                n_prompt_cached: 0, n_prompt_processed: 10,
                prompt_ms: 100.0, prompt_tps: 100.0,
                prompt_per_token_ms: 10.0,
                n_gen: 5, gen_ms: 200.0, gen_tps: 25.0,
                gen_per_token_ms: 40.0,
                n_draft_tokens: 0, n_draft_accepted: 0,
                captured_at: 1000000000,
            },
            TaskInsert {
                task_id: 2, model_load_id: 1, slot_id: 1,
                n_prompt_cached: 0, n_prompt_processed: 20,
                prompt_ms: 200.0, prompt_tps: 100.0,
                prompt_per_token_ms: 10.0,
                n_gen: 10, gen_ms: 400.0, gen_tps: 25.0,
                gen_per_token_ms: 40.0,
                n_draft_tokens: 0, n_draft_accepted: 0,
                captured_at: 1000000000,
            },
        ];
        let count = bulk_insert(&pool, &tasks).await.unwrap();
        assert_eq!(count, 2);
    }

    #[tokio::test]
    #[ignore = "requires database"]
    async fn test_bulk_insert_duplicate_skipped() {
        let pool = test_pool().await;
        let tasks = vec![TaskInsert {
            task_id: 1, model_load_id: 1, slot_id: 0,
            n_prompt_cached: 0, n_prompt_processed: 10,
            prompt_ms: 100.0, prompt_tps: 100.0,
            prompt_per_token_ms: 10.0,
            n_gen: 5, gen_ms: 200.0, gen_tps: 25.0,
            gen_per_token_ms: 40.0,
            n_draft_tokens: 0, n_draft_accepted: 0,
            captured_at: 1000000000,
        }];
        let c1 = bulk_insert(&pool, &tasks).await.unwrap();
        let c2 = bulk_insert(&pool, &tasks).await.unwrap();
        assert_eq!(c1, 1);
        assert_eq!(c2, 0); // duplicate should be ignored
    }

    async fn test_pool() -> SqlitePool {
        let pool = SqlitePool::connect(":memory:").await.unwrap();
        crate::db::schema::init_db(&pool).await.unwrap();
        pool
    }
}
