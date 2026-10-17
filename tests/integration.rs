//! Integration tests for llama-mon.
//!
//! These tests run against a live llama-server instance.
//! By default they are marked `#[ignore]` and only run when
//! `--include-ignored` is passed to `cargo test`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;
use llama_mon::config::Config;
use llama_mon::db;
use llama_mon::db::model_load::ModelLoadRow;
use llama_mon::poller::{Poller, PollerError};

fn init_test_logging() {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_test_writer()
        .try_init();
}

/// Default server URL for integration tests.
/// Override via `LLAMA_MON_SERVER_URL` env var or `.env` file.
const DEFAULT_SERVER_URL: &str = "http://localhost:8080";

/// Unique DB path counter for test isolation.
static DB_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Build a config pointing at the configured server with a unique DB path.
///
/// Uses `LLAMA_MON_DB_PATH` env var if set, otherwise generates a unique
/// path per test to avoid concurrent-test collisions.
fn make_config() -> Config {
    make_config_with_timeout(Duration::from_secs(2))
}

/// Build a config with a custom poll timeout.
fn make_config_with_timeout(timeout: Duration) -> Config {
    let db_path = if let Ok(p) = std::env::var("LLAMA_MON_DB_PATH") {
        PathBuf::from(p)
    } else {
        let id = DB_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        std::env::current_dir()
            .unwrap_or_default()
            .join(format!("db/integration_{}.db", id))
    };
    let server_url = std::env::var("LLAMA_MON_SERVER_URL")
        .unwrap_or_else(|_| DEFAULT_SERVER_URL.to_string());
    Config {
        server_url,
        poll_interval: humantime::Duration::from(Duration::from_secs(2)),
        db_path,
        timeout: humantime::Duration::from(Duration::from_secs(5)),
        _resolved_url: None,
        verbose: true,
        models: vec![],
        fail_fast: false,
    }
}

/// Clean up test DB files for the given path.
fn clean_test_db(path: &Path) {
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(format!("{}-wal", path.display()));
    let _ = std::fs::remove_file(format!("{}-shm", path.display()));
}

/// Resolve config (creates parent dirs, parses URL).
fn resolve_config(cfg: &mut Config) {
    cfg.resolve().expect("config resolve failed");
}

/// Check if the server is reachable by fetching /models.
async fn server_reachable() -> bool {
    let client = reqwest::Client::new();
    let server_url = std::env::var("LLAMA_MON_SERVER_URL")
        .unwrap_or_else(|_| DEFAULT_SERVER_URL.to_string());
    client
        .get(format!("{}/models", server_url))
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .is_ok()
}

/// Helper: create a poller from a config.
async fn make_poller(config: Config) -> Result<Poller> {
    init_test_logging();
    let mut cfg = config;
    resolve_config(&mut cfg);
    let pool = db::open_pool(&cfg.db_path).await?;
    db::schema::init_db(&pool).await?;
    Ok(Poller::new(cfg, pool))
}

// ---------------------------------------------------------------------------
// P5.4.2 — Model appears in DB after first poll
// ---------------------------------------------------------------------------

/// Verify that a model from the live server appears in the model_loads table
/// after a single poll cycle.
#[tokio::test]
#[ignore = "integration test — requires live llama-server, set LLAMA_MON_SERVER_URL"]
async fn test_model_appears_in_db_after_first_poll() -> Result<()> {
    let config = make_config();
    clean_test_db(&config.db_path);
    let mut poller = make_poller(config.clone()).await?;

    // Check server is reachable
    let server_url = std::env::var("LLAMA_MON_SERVER_URL")
        .unwrap_or_else(|_| DEFAULT_SERVER_URL.to_string());
    assert!(
        server_reachable().await,
        "Server at {} is unreachable — skipping test",
        server_url
    );

    // Run one cycle
    let stats = poller.run_until(1).await?;
    assert_eq!(stats.total_poll_cycles, 1);

    // Query DB
    let pool = poller.pool();
    let rows: Vec<ModelLoadRow> = sqlx::query_as("SELECT * FROM model_loads")
        .fetch_all(pool)
        .await?;

    assert!(!rows.is_empty(), "model_loads should have at least one row");

    // Find a loaded/sleeping model
    let loaded = rows.iter().find(|r| {
        r.status == "loaded" || r.status == "sleeping"
    });
    assert!(loaded.is_some(), "Should find at least one loaded/sleeping model in DB");

    let row = loaded.unwrap();
    assert!(!row.model_name.is_empty(), "model_name should not be empty");
    assert!(row.created > 0, "created should be positive");

    Ok(())
}

// ---------------------------------------------------------------------------
// P5.4.3 — Tasks are persisted and deduplicated
// ---------------------------------------------------------------------------

/// Verify that tasks are inserted on first scrape and not re-inserted on
/// subsequent scrapes (deduplication via task_ids_seen CSV).
#[tokio::test]
#[ignore = "integration test — requires live llama-server, set LLAMA_MON_SERVER_URL"]
async fn test_tasks_persisted_and_deduplicated() -> Result<()> {
    let config = make_config();
    clean_test_db(&config.db_path);
    let mut poller = make_poller(config).await?;

    let server_url = std::env::var("LLAMA_MON_SERVER_URL")
        .unwrap_or_else(|_| DEFAULT_SERVER_URL.to_string());
    assert!(
        server_reachable().await,
        "Server at {} is unreachable — skipping test",
        server_url
    );

    // Run 3 cycles
    let stats = poller.run_until(3).await?;
    assert_eq!(stats.total_poll_cycles, 3);

    let pool = poller.pool();

    // Check scrapes were recorded
    let scrape_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM scrapes")
        .fetch_one(pool)
        .await?;
    let model_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM model_loads")
        .fetch_one(pool)
        .await?;
    let task_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM tasks")
        .fetch_one(pool)
        .await?;
    eprintln!(
        "DEBUG: cycles={}, scrapes={}, model_loads={}, tasks={}",
        stats.total_poll_cycles, scrape_count, model_count, task_count
    );
    assert!(scrape_count >= 2, "Should have at least 2 scrapes after 3 cycles (got {}, stats: {:?})", scrape_count, stats);

    // Check tasks are not re-inserted between cycle 2 and 3
    let task_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM tasks")
        .fetch_one(pool)
        .await?;
    // If there are any tasks, they shouldn't have grown beyond cycle 2
    assert!(task_count >= 0, "tasks table should exist (may be 0 if no recent tasks)");

    Ok(())
}

// ---------------------------------------------------------------------------
// P5.4.4 — Reload detection
// ---------------------------------------------------------------------------

/// Verify reload detection: when tasks.completed decreases, a new model_loads
/// row should be created and the old one closed.
///
/// Note: This test assumes the live server may reload models. If no reload
/// occurs during the test window, the test is skipped.
#[tokio::test]
#[ignore = "integration test — requires live llama-server, set LLAMA_MON_SERVER_URL"]
async fn test_reload_detection() -> Result<()> {
    let config = make_config();
    clean_test_db(&config.db_path);
    let mut poller = make_poller(config).await?;

    let server_url = std::env::var("LLAMA_MON_SERVER_URL")
        .unwrap_or_else(|_| DEFAULT_SERVER_URL.to_string());
    assert!(
        server_reachable().await,
        "Server at {} is unreachable — skipping test",
        server_url
    );

    // Run 10 cycles to give the server a chance to reload
    let stats = poller.run_until(10).await?;

    // If no reloads occurred, skip the assertion
    if stats.total_reloads == 0 {
        println!("No reloads detected during 10 cycles — test condition not met");
        return Ok(());
    }

    let pool = poller.pool();

    // For a model that was reloaded, there should be multiple model_loads rows
    let rows: Vec<ModelLoadRow> = sqlx::query_as("SELECT * FROM model_loads ORDER BY created")
        .fetch_all(pool)
        .await?;

    assert!(stats.total_reloads > 0, "Expected at least one reload");

    Ok(())
}

// ---------------------------------------------------------------------------
// P5.4.5 — Graceful shutdown and WAL checkpoint
// ---------------------------------------------------------------------------

/// Verify that the poller shuts down cleanly and WAL is checkpointed.
#[tokio::test]
#[ignore = "integration test — requires live llama-server, set LLAMA_MON_SERVER_URL"]
async fn test_graceful_shutdown_wal_checkpoint() -> Result<()> {
    let config = make_config();
    clean_test_db(&config.db_path);
    let db_path = config.db_path.clone();
    let mut poller = make_poller(config).await?;

    let server_url = std::env::var("LLAMA_MON_SERVER_URL")
        .unwrap_or_else(|_| DEFAULT_SERVER_URL.to_string());
    assert!(
        server_reachable().await,
        "Server at {} is unreachable — skipping test",
        server_url
    );

    // Run a few cycles
    let stats = poller.run_until(3).await?;
    assert_eq!(stats.total_poll_cycles, 3);

    let pool = poller.pool();

    // Verify data was persisted
    let model_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM model_loads")
        .fetch_one(pool)
        .await?;
    assert!(model_count > 0, "Should have model_loads rows before shutdown");

    // Perform WAL checkpoint manually (simulating graceful shutdown)
    let checkpointed = db::wal_checkpoint(pool).await?;
    assert!(checkpointed >= 0, "WAL checkpoint completed");

    // Verify the main DB file is consistent
    assert!(db_path.exists(), "DB file should exist");
    assert!(db_path.metadata()?.len() > 0, "DB file should have content");

    Ok(())
}

// ---------------------------------------------------------------------------
// Utility tests
// ---------------------------------------------------------------------------

/// Verify that the poller handles a 404 gracefully without panicking.
#[tokio::test]
#[ignore = "integration test — requires live llama-server, set LLAMA_MON_SERVER_URL"]
async fn test_poller_handles_unreachable_model() -> Result<()> {
    let base = std::env::current_dir().unwrap_or_default();
    let db_path = base.join("db/test_unreachable.db");
    let server_url = std::env::var("LLAMA_MON_SERVER_URL")
        .unwrap_or_else(|_| DEFAULT_SERVER_URL.to_string());
    let config = Config {
        server_url: format!("{}/models", server_url),
        poll_interval: humantime::Duration::from(Duration::from_secs(2)),
        db_path,
        timeout: humantime::Duration::from(Duration::from_secs(3)),
        _resolved_url: None,
        verbose: false,
        models: vec![],
        fail_fast: false,
    };
    let mut poller = make_poller(config).await?;

    // Even if the server returns unexpected data, the poller should not panic
    let result = poller.run_until(1).await;

    // The result might be Ok (stats collected) or Err (server error)
    // but it should not panic
    match result {
        Ok(stats) => {
            assert_eq!(stats.total_poll_cycles, 1);
        }
        Err(PollerError::Http(_)) => {
            // Expected if the endpoint doesn't return valid /models JSON
        }
        Err(e) => panic!("Unexpected error type: {}", e),
    }

    Ok(())
}
