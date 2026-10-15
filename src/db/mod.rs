//! Database persistence layer for llama-mon.
//!
//! Provides SQLite access for model loads, tasks, and scrape records.
//! All DDL lives in `schema.rs`; this module handles connection lifecycle.

pub mod model_load;
pub mod scrape;
pub mod schema;
pub mod task;

use sqlx::{Connection, SqlitePool, SqliteConnection};
use std::path::Path;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum DbError {
    #[error("SQLx error: {0}")]
    Sqlx(#[from] sqlx::Error),
    #[error("Database I/O: {0}")]
    Io(#[from] std::io::Error),
}

/// Re-export sqlx for convenience.
pub use sqlx;

/// Open a connection pool to the SQLite database at the given path.
/// Enables WAL mode and busy timeout automatically.
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

/// Open a single connection to the SQLite database at the given path.
/// Enables WAL mode and busy timeout.
pub async fn open_db(path: &Path) -> Result<SqliteConnection, DbError> {
    std::fs::create_dir_all(path.parent().expect("db_path must have a parent"))?;

    let dsn = format!("sqlite:{}", path.display());
    let mut conn = SqliteConnection::connect(&dsn).await?;

    sqlx::query("PRAGMA journal_mode=WAL")
        .execute(&mut conn)
        .await?;
    sqlx::query("PRAGMA busy_timeout=5000")
        .execute(&mut conn)
        .await?;

    Ok(conn)
}

/// Perform a WAL checkpoint to flush pending pages to the main database.
/// Returns the number of frames checkpointed.
pub async fn wal_checkpoint(pool: &SqlitePool) -> Result<i64, DbError> {
    let result: i64 = sqlx::query_scalar("PRAGMA wal_checkpoint(PASSIVE)")
        .fetch_one(pool)
        .await?;
    Ok(result)
}

/// Execute a database operation with retry on transient errors (BUSY/LOCKED).
///
/// Retries up to `max_attempts` times, waiting `100ms * attempt` between retries.
/// Logs a warning on each retry.
pub async fn retry_with_backoff<F, T>(f: F, max_attempts: u32) -> Result<T, DbError>
where
    F: Fn() -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<T, DbError>> + Send>>,
{
    let mut attempts = 0u32;
    loop {
        attempts += 1;
        match f().await {
            Ok(v) => return Ok(v),
            Err(e) => {
                if attempts >= max_attempts {
                    return Err(e);
                }
                if e.is_transient() {
                    tracing::warn!(
                        attempt = attempts,
                        max_attempts = max_attempts,
                        "Transient DB error, retrying"
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(100 * attempts as u64)).await;
                    continue;
                }
                return Err(e);
            }
        }
    }
}

/// Check if a `DbError` represents a transient/busy condition.
impl DbError {
    /// Returns true if this error is transient (BUSY, LOCKED, etc.).
    pub fn is_transient(&self) -> bool {
        match self {
            DbError::Sqlx(sqlx::Error::Database(e)) => {
                e.code().map(|c| c == "5" || c == "6").unwrap_or(false)
                // SQLite BUSY=5, LOCKED=6
            }
            _ => false,
        }
    }
}
