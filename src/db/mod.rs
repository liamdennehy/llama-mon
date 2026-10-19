//! Database persistence layer for llama-mon.
//!
//! Provides SQLite access for model loads, tasks, and scrape records.
//! All DDL lives in `schema.rs`; this module handles connection lifecycle.

pub mod model_load;
pub mod scrape;
pub mod schema;
pub mod task;

/// Shared mutex to serialize real-file DB test pool creation.
/// Prevents "unable to open database file" races when multiple tests
/// try to create temp directories and SQLite files concurrently.
pub(crate) static DB_TEST_POOL_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

use sqlx::{Connection, SqlitePool, SqliteConnection};
use sqlx::sqlite::{SqliteConnectOptions};
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
///
/// Relative paths are resolved against the current working directory to
/// avoid SQLite's inability to open relative paths from certain contexts.
///
/// The parent directory must already exist; this function does not create it.
pub async fn open_pool(path: &Path) -> Result<SqlitePool, DbError> {
    if path.as_os_str().is_empty() || path.to_str() == Some(":memory:") {
        // :memory: uses the string DSN directly
        let dsn = path.to_str().unwrap_or(":memory:");
        let pool = SqlitePool::connect(dsn).await?;
        // Enable WAL mode (no-op for :memory:, ignored on failure)
        let _ = sqlx::query("PRAGMA journal_mode=WAL")
            .execute(&pool)
            .await;
        sqlx::query("PRAGMA busy_timeout=5000")
            .execute(&pool)
            .await?;
        return Ok(pool);
    }

    // Resolve to absolute path for file-based databases.
    let abs_path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };

    // Use SqliteConnectOptions with create_if_missing(true) so that
    // the .db file is created automatically when the pool opens.
    // SqlitePool::connect("sqlite:/path") will fail with SQLITE_CANTOPEN
    // if the file does not already exist.
    let opts = SqliteConnectOptions::new()
        .filename(&abs_path)
        .create_if_missing(true);
    let pool = SqlitePool::connect_with(opts).await?;

    // Enable WAL mode for better concurrent read performance.
    let _ = sqlx::query("PRAGMA journal_mode=WAL")
        .execute(&pool)
        .await;

    // Increase busy timeout to avoid "database is locked" panics
    sqlx::query("PRAGMA busy_timeout=5000")
        .execute(&pool)
        .await?;

    Ok(pool)
}

/// Open a single connection to the SQLite database at the given path.
/// Enables WAL mode and busy timeout.
pub async fn open_db(path: &Path) -> Result<SqliteConnection, DbError> {
    if path.as_os_str().is_empty() || path.to_str() == Some(":memory:") {
        let dsn = path.to_str().unwrap_or(":memory:");
        let mut conn = SqliteConnection::connect(dsn).await?;
        sqlx::query("PRAGMA journal_mode=WAL")
            .execute(&mut conn)
            .await?;
        sqlx::query("PRAGMA busy_timeout=5000")
            .execute(&mut conn)
            .await?;
        return Ok(conn);
    }

    let abs_path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };

    let opts = SqliteConnectOptions::new()
        .filename(&abs_path)
        .create_if_missing(true);
    let mut conn = SqliteConnection::connect_with(&opts).await?;

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Helper: create a unique temp directory and return both the TempDir
    /// (for automatic cleanup on drop, regardless of panic) and the db path.
    fn make_test_db() -> (tempfile::TempDir, PathBuf) {
        let _guard = crate::db::DB_TEST_POOL_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        (dir, db_path)
    }

    /// Test that open_pool + init_db works on a real temp file.
    #[tokio::test]
    async fn test_open_pool_creates_file() {
        let (_dir, db_path) = make_test_db();

        let pool = open_pool(&db_path).await.unwrap();
        // Verify we can actually use the pool
        let result: i64 = sqlx::query_scalar("SELECT 42")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(result, 42);

        // Verify init_db creates tables
        schema::init_db(&pool).await.unwrap();
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table'"
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(count > 0, "tables should exist after init_db");

        // WAL mode should be active after init_db
        let mode: String = sqlx::query_scalar("PRAGMA journal_mode")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(mode, "wal", "journal mode should be WAL");

        // _dir dropped here → temp dir + db files cleaned up automatically
    }

    /// Test that open_db creates a usable file connection.
    #[tokio::test]
    async fn test_open_db_creates_file() {
        let (_dir, db_path) = make_test_db();

        let mut conn = open_db(&db_path).await.unwrap();
        let result: i64 = sqlx::query_scalar("SELECT 42")
            .fetch_one(&mut conn)
            .await
            .unwrap();
        assert_eq!(result, 42);

        // Verify DB file was created on disk
        assert!(db_path.exists(), "DB file should exist on disk");

        // _dir dropped here → temp dir + db files cleaned up automatically
    }

    /// Test that open_pool fails when parent directory does not exist.
    #[tokio::test]
    async fn test_open_pool_fails_without_parent_dir() {
        // Use a path deep in /proc or /dev which is non-createable
        let nonexistent = Path::new("/tmp/nonexistent-parent-dir-xyz/db/test.db");
        let result = open_pool(nonexistent).await;
        assert!(result.is_err(), "open_pool should fail when parent dir missing");
    }

}
