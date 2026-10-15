//! DDL statements for llama-mon.
//!
//! All table and index creation lives here. Called once during `init_db`.

use sqlx::{Executor, SqlitePool};

use crate::db::DbError;

/// Current schema version. Increment on DDL changes.
pub const VERSION: &str = "1";

/// Expected version string stored in meta table.
const EXPECTED_VERSION: &str = VERSION;

pub const SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS model_loads (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    model_name        TEXT    NOT NULL,
    created           INTEGER NOT NULL,
    status            TEXT    NOT NULL DEFAULT 'unknown',
    loaded_at         INTEGER,
    unloaded_at       INTEGER,
    preset_raw        TEXT,
    ctx_size          INTEGER,
    n_batch           INTEGER,
    n_ubatch          INTEGER,
    parallel          INTEGER,
    threads           INTEGER,
    threads_batch     INTEGER,
    n_gpu_layers      INTEGER,
    cache_type_k      TEXT,
    cache_type_v      TEXT,
    flash_attn        INTEGER,
    kv_unified        INTEGER,
    context_bytes     INTEGER,
    model_bytes       INTEGER,
    scrape_count      INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_model_loads_model_created
    ON model_loads (model_name, created);

CREATE TABLE IF NOT EXISTS tasks (
    task_id           INTEGER NOT NULL,
    model_load_id     INTEGER NOT NULL REFERENCES model_loads(id),
    slot_id           INTEGER NOT NULL,
    n_prompt_cached       INTEGER DEFAULT 0,
    n_prompt_processed    INTEGER NOT NULL,
    prompt_ms             REAL    NOT NULL,
    prompt_tps            REAL,
    prompt_per_token_ms   REAL,
    n_gen             INTEGER NOT NULL,
    gen_ms            REAL    NOT NULL,
    gen_tps           REAL,
    gen_per_token_ms  REAL,
    n_draft_tokens      INTEGER DEFAULT 0,
    n_draft_accepted    INTEGER DEFAULT 0,
    captured_at         INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_tasks_model_load
    ON tasks (model_load_id);
CREATE INDEX IF NOT EXISTS idx_tasks_slot
    ON tasks (model_load_id, slot_id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_tasks_unique
    ON tasks (task_id, model_load_id);

CREATE TABLE IF NOT EXISTS scrapes (
    id                      INTEGER PRIMARY KEY AUTOINCREMENT,
    model_name              TEXT    NOT NULL,
    model_created           INTEGER NOT NULL,
    model_load_id           INTEGER NOT NULL REFERENCES model_loads(id),
    scraped_at              INTEGER NOT NULL,
    context_max             INTEGER,
    tasks_processing        INTEGER DEFAULT 0,
    tasks_queued            INTEGER DEFAULT 0,
    tasks_completed         INTEGER DEFAULT 0,
    tasks_duration_s        REAL,
    prompt_tokens_total     INTEGER,
    prompt_tokens_cached    INTEGER,
    prompt_tokens_processed INTEGER,
    prediction_tokens_total INTEGER,
    prediction_tokens_processed INTEGER,
    decode_total            INTEGER,
    kvcache_active_tokens   INTEGER,
    memory_context_bytes    INTEGER,
    memory_model_bytes      INTEGER,
    task_ids_seen           TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_scrapes_model_created
    ON scrapes (model_name, model_created, scraped_at);
"#;

/// Initialize the database: create tables, indexes, and the version row.
/// Returns the effective schema version (new or existing).
pub async fn init_db(pool: &SqlitePool) -> Result<String, DbError> {
    // Run all DDL
    for statement in SCHEMA_SQL.split(';') {
        let statement = statement.trim();
        if !statement.is_empty() {
            sqlx::query(statement).execute(pool).await?;
        }
    }

    // Insert version if not present
    sqlx::query("INSERT OR IGNORE INTO meta (key, value) VALUES ('version', ?)")
        .bind(VERSION)
        .execute(pool)
        .await?;

    Ok(VERSION.to_string())
}

/// Initialize the database using a raw connection (no pool).
/// Useful for single-connection operations.
pub async fn init_db_with_conn(conn: &mut sqlx::SqliteConnection) -> Result<String, DbError> {
    // Run all DDL
    for statement in SCHEMA_SQL.split(';') {
        let statement = statement.trim();
        if !statement.is_empty() {
            conn.execute(statement).await?;
        }
    }

    // Insert version if not present
    sqlx::query("INSERT OR IGNORE INTO meta (key, value) VALUES ('version', ?)")
        .bind(VERSION)
        .execute(conn)
        .await?;

    Ok(VERSION.to_string())
}

/// Check the stored schema version against the expected version.
/// Returns None if no version is set (fresh DB) or table doesn't exist.
pub async fn check_version(pool: &SqlitePool) -> Result<Option<String>, DbError> {
    let result: Result<Option<String>, sqlx::Error> = sqlx::query_scalar(
        "SELECT value FROM meta WHERE key = 'version'"
    )
    .fetch_optional(pool)
    .await;
    // If the meta table doesn't exist yet, return None (fresh DB)
    match result {
        Ok(v) => Ok(v),
        Err(sqlx::Error::Database(db_err))
            if db_err.code() == Some("1".into()) =>
        {
            Ok(None)
        }
        Err(e) => Err(DbError::Sqlx(e)),
    }
}

/// Check version and log a warning if mismatched.
pub async fn verify_version(pool: &SqlitePool) -> Result<(), DbError> {
    match check_version(pool).await? {
        Some(stored) if stored != EXPECTED_VERSION => {
            tracing::warn!(
                stored = %stored,
                expected = %EXPECTED_VERSION,
                "Schema version mismatch — migration may be required"
            );
        }
        Some(stored) => {
            tracing::debug!(version = %stored, "Schema version OK");
        }
        None => {
            tracing::debug!("No schema version found (fresh database)");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_init_db_creates_tables() {
        let pool = SqlitePool::connect(":memory:").await.unwrap();
        let version = init_db(&pool).await.unwrap();
        assert_eq!(version, "1");

        // Verify tables exist
        let meta: Option<String> = sqlx::query_scalar(
            "SELECT value FROM meta WHERE key = 'version'"
        )
        .fetch_optional(&pool)
        .await
        .unwrap();
        assert_eq!(meta, Some("1".to_string()));
    }

    #[tokio::test]
    async fn test_verify_version_ok() {
        let pool = SqlitePool::connect(":memory:").await.unwrap();
        init_db(&pool).await.unwrap();
        // Should not warn
        verify_version(&pool).await.unwrap();
    }

    #[tokio::test]
    async fn test_check_version_none_fresh() {
        let pool = SqlitePool::connect(":memory:").await.unwrap();
        // Don't call init_db — should return None
        let version = check_version(&pool).await.unwrap();
        assert!(version.is_none());
    }
}
