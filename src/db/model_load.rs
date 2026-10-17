//! model_loads table CRUD operations.

use sqlx::SqlitePool;

use crate::db::DbError;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ModelLoadRow {
    pub id: i64,
    pub model_name: String,
    pub created: i64,
    pub status: String,
    pub loaded_at: Option<i64>,
    pub unloaded_at: Option<i64>,
    pub preset_raw: Option<String>,
    pub ctx_size: Option<i64>,
    pub n_batch: Option<i64>,
    pub n_ubatch: Option<i64>,
    pub parallel: Option<i64>,
    pub threads: Option<i64>,
    pub threads_batch: Option<i64>,
    pub n_gpu_layers: Option<i64>,
    pub cache_type_k: Option<String>,
    pub cache_type_v: Option<String>,
    pub flash_attn: Option<i64>,
    pub kv_unified: Option<i64>,
    pub context_bytes: Option<i64>,
    pub model_bytes: Option<i64>,
    pub scrape_count: i64,
}

#[derive(Debug, Clone)]
pub struct ModelLoadInsert<'a> {
    pub model_name: &'a str,
    pub created: i64,
    pub status: &'a str,
    pub preset_raw: Option<&'a str>,
    pub ctx_size: Option<i64>,
    pub n_batch: Option<i64>,
    pub n_ubatch: Option<i64>,
    pub parallel: Option<i64>,
    pub threads: Option<i64>,
    pub threads_batch: Option<i64>,
    pub n_gpu_layers: Option<i64>,
    pub cache_type_k: Option<&'a str>,
    pub cache_type_v: Option<&'a str>,
    pub flash_attn: Option<i64>,
    pub kv_unified: Option<i64>,
    pub context_bytes: Option<i64>,
    pub model_bytes: Option<i64>,
}

/// Upsert a model load. Returns the existing or newly inserted id.
pub async fn upsert(
    pool: &SqlitePool,
    ml: &ModelLoadInsert<'_>,
) -> Result<i64, DbError> {
    let existing = sqlx::query_scalar::<_, i64>(
        "SELECT id FROM model_loads WHERE model_name = ? AND created = ?"
    )
    .bind(ml.model_name)
    .bind(ml.created)
    .fetch_optional(pool)
    .await?;

    if let Some(id) = existing {
        sqlx::query(
            r#"UPDATE model_loads SET
                status = ?, preset_raw = ?, ctx_size = ?, n_batch = ?, n_ubatch = ?,
                parallel = ?, threads = ?, threads_batch = ?, n_gpu_layers = ?,
                cache_type_k = ?, cache_type_v = ?,
                flash_attn = ?, kv_unified = ?,
                context_bytes = ?, model_bytes = ?
            WHERE id = ?"#
        )
        .bind(ml.status)
        .bind(ml.preset_raw)
        .bind(ml.ctx_size)
        .bind(ml.n_batch)
        .bind(ml.n_ubatch)
        .bind(ml.parallel)
        .bind(ml.threads)
        .bind(ml.threads_batch)
        .bind(ml.n_gpu_layers)
        .bind(ml.cache_type_k)
        .bind(ml.cache_type_v)
        .bind(ml.flash_attn)
        .bind(ml.kv_unified)
        .bind(ml.context_bytes)
        .bind(ml.model_bytes)
        .bind(id)
        .execute(pool)
        .await?;
        Ok(id)
    } else {
        let id = sqlx::query_scalar(
            r#"INSERT INTO model_loads (
                model_name, created, status, preset_raw, ctx_size, n_batch, n_ubatch,
                parallel, threads, threads_batch, n_gpu_layers,
                cache_type_k, cache_type_v,
                flash_attn, kv_unified,
                context_bytes, model_bytes
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            RETURNING id"#
        )
        .bind(ml.model_name)
        .bind(ml.created)
        .bind(ml.status)
        .bind(ml.preset_raw)
        .bind(ml.ctx_size)
        .bind(ml.n_batch)
        .bind(ml.n_ubatch)
        .bind(ml.parallel)
        .bind(ml.threads)
        .bind(ml.threads_batch)
        .bind(ml.n_gpu_layers)
        .bind(ml.cache_type_k)
        .bind(ml.cache_type_v)
        .bind(ml.flash_attn)
        .bind(ml.kv_unified)
        .bind(ml.context_bytes)
        .bind(ml.model_bytes)
        .fetch_one(pool)
        .await?;
        Ok(id)
    }
}

/// Mark a model load as unloaded.
pub async fn close(pool: &SqlitePool, id: i64, now: i64) -> Result<(), DbError> {
    sqlx::query(
        "UPDATE model_loads SET status = 'unloaded', unloaded_at = ? WHERE id = ?"
    )
    .bind(now)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Update status and optionally set loaded_at.
pub async fn update_status(
    pool: &SqlitePool,
    id: i64,
    status: &str,
    loaded_at: Option<i64>,
) -> Result<(), DbError> {
    match loaded_at {
        Some(la) => {
            sqlx::query(
                "UPDATE model_loads SET status = ?, loaded_at = ?, unloaded_at = NULL WHERE id = ?"
            )
            .bind(status)
            .bind(la)
            .bind(id)
            .execute(pool)
            .await?;
        }
        None => {
            sqlx::query("UPDATE model_loads SET status = ? WHERE id = ?")
                .bind(status)
                .bind(id)
                .execute(pool)
                .await?;
        }
    }
    Ok(())
}

/// Increment the scrape_count for a model load.
pub async fn increment_scrape_count(pool: &SqlitePool, id: i64) -> Result<(), DbError> {
    sqlx::query(
        "UPDATE model_loads SET scrape_count = scrape_count + 1 WHERE id = ?"
    )
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Get all model_load ids for a given model name.
pub async fn get_ids_by_name(pool: &SqlitePool, model_name: &str) -> Result<Vec<i64>, DbError> {
    let ids: Vec<i64> = sqlx::query_scalar(
        "SELECT id FROM model_loads WHERE model_name = ?"
    )
    .bind(model_name)
    .fetch_all(pool)
    .await?;
    Ok(ids)
}

/// Fetch a single model_load row by id.
pub async fn get_by_id(pool: &SqlitePool, id: i64) -> Result<Option<ModelLoadRow>, DbError> {
    let row: Option<ModelLoadRow> = sqlx::query_as(
        r#"SELECT id, model_name, created, status, loaded_at, unloaded_at,
                preset_raw, ctx_size, n_batch, n_ubatch, parallel, threads,
                threads_batch, n_gpu_layers, cache_type_k, cache_type_v,
                flash_attn, kv_unified, context_bytes, model_bytes, scrape_count
           FROM model_loads WHERE id = ?"#
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

#[cfg(test)]
mod tests {
    use super::*;

    // These tests require a real SQLite database.
    // They are skipped when no DB is available.
    // A test container or in-memory SQLite would make them runnable.

    #[tokio::test]
    #[ignore = "requires database"]
    async fn test_upsert_new_load() {
        let pool = test_pool().await;
        let insert = ModelLoadInsert {
            model_name: "test-model",
            created: 9999999999,
            status: "loaded",
            preset_raw: Some("[test]\nctx-size = 1024"),
            ctx_size: Some(1024),
            n_batch: None,
            n_ubatch: None,
            parallel: Some(2),
            threads: Some(4),
            threads_batch: None,
            n_gpu_layers: Some(-1),
            cache_type_k: Some("f16"),
            cache_type_v: Some("f16"),
            flash_attn: Some(1),
            kv_unified: Some(1),
            context_bytes: Some(1000),
            model_bytes: Some(2000),
        };
        let id = upsert(&pool, &insert).await.unwrap();
        assert!(id > 0);
    }

    #[tokio::test]
    #[ignore = "requires database"]
    async fn test_upsert_existing_load() {
        let pool = test_pool().await;
        let insert = ModelLoadInsert {
            model_name: "test-model-dup",
            created: 8888888888,
            status: "loaded",
            preset_raw: Some("[test]"),
            ctx_size: Some(1024),
            n_batch: None,
            n_ubatch: None,
            parallel: None,
            threads: None,
            threads_batch: None,
            n_gpu_layers: None,
            cache_type_k: None,
            cache_type_v: None,
            flash_attn: None,
            kv_unified: None,
            context_bytes: None,
            model_bytes: None,
        };
        let id1 = upsert(&pool, &insert).await.unwrap();
        let id2 = upsert(&pool, &insert).await.unwrap();
        assert_eq!(id1, id2);
    }

    #[tokio::test]
    #[ignore = "requires database"]
    async fn test_get_by_id() {
        let pool = test_pool().await;
        let insert = ModelLoadInsert {
            model_name: "test-get",
            created: 7777777777,
            status: "loaded",
            preset_raw: Some("[get]"),
            ctx_size: Some(4096),
            n_batch: None, n_ubatch: None, parallel: None, threads: None,
            threads_batch: None, n_gpu_layers: None, cache_type_k: None,
            cache_type_v: None, flash_attn: None, kv_unified: None,
            context_bytes: None, model_bytes: None,
        };
        let id = upsert(&pool, &insert).await.unwrap();

        let row = get_by_id(&pool, id).await.unwrap().unwrap();
        assert_eq!(row.id, id);
        assert_eq!(row.model_name, "test-get");
        assert_eq!(row.ctx_size, Some(4096));

        let none = get_by_id(&pool, 9999).await.unwrap();
        assert!(none.is_none());
    }

    async fn test_pool() -> SqlitePool {
        let pool = SqlitePool::connect(":memory:").await.unwrap();
        crate::db::schema::init_db(&pool).await.unwrap();
        pool
    }
}
