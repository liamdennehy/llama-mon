//! DDL statements for llama-mon.
//!
//! All table and index creation lives here. Called once during `init_db`.

use sqlx::SqlitePool;

use crate::db::DbError;

pub const VERSION: &str = "1";

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
    model_path        TEXT,
    log_verbosity     INTEGER,
    flash_attn        INTEGER,
    kv_unified        INTEGER,
    jinja             INTEGER,
    cache_prompt      INTEGER,
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
pub async fn init_db(pool: &SqlitePool) -> Result<(), DbError> {
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

    Ok(())
}
