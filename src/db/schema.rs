//! DDL statements for llama-mon.
//!
//! All table and index creation lives here. Called once during `init_db`.

use sqlx::{Executor, SqlitePool};

use crate::db::DbError;

/// Current schema version. Increment on DDL changes.
pub const VERSION: &str = "4";

/// Add convenience views for CLI analysis and simplified queries.
pub const VIEWS_SQL: &str = r#"
-- Tasks with model metadata and computed context length.
-- Used by: task.rs get_recent(), dashboard bubble chart, CLI analysis.
CREATE VIEW IF NOT EXISTS v_task_summary AS
SELECT
    t.task_id,
    m.model_name,
    m.ctx_size        AS model_ctx_size,
    m.threads         AS model_threads,
    m.parallel        AS model_parallel,
    m.created         AS model_created,
    t.slot_id,
    t.n_prompt_cached,
    t.n_prompt_processed,
    t.n_prompt_cached + t.n_prompt_processed AS context_length,
    t.prompt_ms,
    t.prompt_tps,
    t.prompt_per_token_ms,
    t.n_gen,
    t.gen_ms,
    t.gen_tps,
    t.gen_per_token_ms,
    t.n_draft_tokens,
    t.n_draft_accepted,
    t.captured_at
FROM tasks t
JOIN model_loads m ON t.model_load_id = m.id;

-- Scrapes with model metadata.
-- Used by: scrape.rs get_recent(), dashboard charts, CLI analysis.
CREATE VIEW IF NOT EXISTS v_scrape_summary AS
SELECT
    s.id,
    s.model_name,
    s.model_created,
    s.model_load_id,
    s.scraped_at,
    s.context_max,
    s.tasks_processing,
    s.tasks_queued,
    s.tasks_completed,
    s.tasks_duration_s,
    s.prompt_tokens_total,
    s.prompt_tokens_cached,
    s.prompt_tokens_processed,
    s.prediction_tokens_total,
    s.prediction_tokens_processed,
    s.decode_total,
    s.kvcache_active_tokens,
    s.memory_context_bytes,
    s.memory_model_bytes,
    s.task_ids_seen,
    m.ctx_size        AS model_ctx_size,
    m.threads         AS model_threads,
    m.parallel        AS model_parallel,
    m.status          AS model_status
FROM scrapes s
JOIN model_loads m ON s.model_load_id = m.id;

-- Per-model-load performance aggregates.
-- Used by: CLI analysis, comparing configs, schema.md examples.
CREATE VIEW IF NOT EXISTS v_model_stats AS
SELECT
    m.id              AS model_load_id,
    m.model_name,
    m.ctx_size        AS model_ctx_size,
    m.threads         AS model_threads,
    m.parallel        AS model_parallel,
    m.status          AS model_status,
    m.created,
    COUNT(t.task_id)  AS task_count,
    ROUND(AVG(t.gen_tps), 2)  AS avg_gen_tps,
    ROUND(AVG(t.prompt_tps), 2) AS avg_prompt_tps,
    ROUND(PERCENTILE_CONT(t.gen_tps, 0.95), 2) AS p95_gen_tps,
    ROUND(PERCENTILE_CONT(t.gen_tps, 0.50), 2) AS median_gen_tps,
    ROUND(AVG(t.n_gen), 0)    AS avg_gen_tokens,
    ROUND(AVG(t.context_length), 0) AS avg_context_length,
    SUM(t.n_gen)      AS total_gen_tokens
FROM model_loads m
LEFT JOIN v_task_summary t ON t.model_load_id = m.id
GROUP BY m.id;

-- Model load history with duration and task counts.
-- Used by: CLI analysis, understanding reload patterns.
CREATE VIEW IF NOT EXISTS v_model_history AS
SELECT
    m.id              AS model_load_id,
    m.model_name,
    m.created,
    m.status          AS model_status,
    m.loaded_at,
    m.unloaded_at,
    m.ctx_size        AS model_ctx_size,
    m.threads         AS model_threads,
    m.parallel        AS model_parallel,
    CASE WHEN m.unloaded_at IS NOT NULL THEN m.unloaded_at - m.loaded_at
         ELSE julianday('now') * 86400 - m.loaded_at END AS session_duration_s,
    COALESCE(
        (SELECT COUNT(*) FROM tasks t WHERE t.model_load_id = m.id), 0
    ) AS task_count
FROM model_loads m
ORDER BY m.created DESC;
"#;

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
    -- Speculative decoding knobs
    cache_ram         INTEGER,
    cache_reuse       INTEGER,
    checkpoint_min_step INTEGER,
    draft_p_min       REAL,
    image_min_tokens  INTEGER,
    load_mode         TEXT,
    op_offload        TEXT,
    reasoning         INTEGER,
    reasoning_budget  INTEGER,
    reasoning_format  TEXT,
    spec_draft_n_max  INTEGER,
    spec_type         TEXT,
    swa_checkpoints   INTEGER
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

/// Initialize the database: create tables, indexes, views, and the version row.
/// Returns the effective schema version (new or existing).
pub async fn init_db(pool: &SqlitePool) -> Result<String, DbError> {
    // Run table DDL
    for statement in SCHEMA_SQL.split(';') {
        let statement = statement.trim();
        if !statement.is_empty() {
            sqlx::query(statement).execute(pool).await?;
        }
    }

    // Run view DDL
    for statement in VIEWS_SQL.split(';') {
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
    // Run table DDL
    for statement in SCHEMA_SQL.split(';') {
        let statement = statement.trim();
        if !statement.is_empty() {
            conn.execute(statement).await?;
        }
    }

    // Run view DDL
    for statement in VIEWS_SQL.split(';') {
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
    use std::path::PathBuf;
    use sqlx::sqlite::SqliteConnectOptions;

    /// NOTE: `:memory:` is forbidden in these tests — it bypasses file creation,
    /// directory permissions, and WAL setup, producing false positives.
    /// All tests use a real SQLite file in a unique temp directory.

    /// Helper: create a unique temp directory and return both the TempDir
    /// (auto-cleaned on drop) and the db path.
    fn make_test_db() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        (dir, db_path)
    }

    #[tokio::test]
    async fn test_init_db_creates_tables() {
        let (_dir, db_path) = make_test_db();
        let opts = SqliteConnectOptions::new()
            .filename(&db_path)
            .create_if_missing(true)
            .read_only(false);
        let pool = SqlitePool::connect_with(opts).await.unwrap();

        let version = init_db(&pool).await.unwrap();
        assert_eq!(version, "4");

        assert!(db_path.exists(), "DB file should exist on disk");

        let meta: Option<String> = sqlx::query_scalar(
            "SELECT value FROM meta WHERE key = 'version'"
        )
        .fetch_optional(&pool)
        .await
        .unwrap();
        assert_eq!(meta, Some("4".to_string()));

        let view_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='view' AND name IN ('v_task_summary','v_scrape_summary','v_model_stats','v_model_history')"
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(view_count, 4);
    }

    #[tokio::test]
    async fn test_verify_version_ok() {
        let (_dir, db_path) = make_test_db();
        let opts = SqliteConnectOptions::new()
            .filename(&db_path)
            .create_if_missing(true)
            .read_only(false);
        let pool = SqlitePool::connect_with(opts).await.unwrap();
        init_db(&pool).await.unwrap();
        verify_version(&pool).await.unwrap();
    }

    #[tokio::test]
    async fn test_check_version_none_fresh() {
        let (_dir, db_path) = make_test_db();
        let opts = SqliteConnectOptions::new()
            .filename(&db_path)
            .create_if_missing(true)
            .read_only(false);
        let pool = SqlitePool::connect_with(opts).await.unwrap();
        let version = check_version(&pool).await.unwrap();
        assert!(version.is_none());
    }
}


