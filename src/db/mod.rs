//! Database persistence layer for llama-mon.
//!
//! Provides SQLite access for model loads, tasks, and scrape records.
//! All DDL lives in `schema.rs`; this module handles connection lifecycle.

pub mod model_load;
pub mod scrape;
pub mod schema;
pub mod task;

use sqlx::SqlitePool;
use std::path::Path;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum DbError {
    #[error("SQLx error: {0}")]
    Sqlx(#[from] sqlx::Error),
    #[error("Database I/O: {0}")]
    Io(#[from] std::io::Error),
}

/// Open a connection pool to the SQLite database at the given path.
/// Enables WAL mode automatically.
pub async fn open_pool(path: &Path) -> Result<SqlitePool, DbError> {
    std::fs::create_dir_all(path.parent().expect("db_path must have a parent"))?;

    let dsn = format!("sqlite:{}", path.display());
    let pool = SqlitePool::connect(&dsn).await?;

    // Enable WAL mode for better concurrent read performance
    sqlx::query("PRAGMA journal_mode=WAL")
        .execute(&pool)
        .await?;

    // Increase busy timeout to avoid "database is locked" panics
    sqlx::query("PRAGMA busy_timeout=5000")
        .execute(&pool)
        .await?;

    Ok(pool)
}
