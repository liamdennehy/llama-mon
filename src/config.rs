//! Configuration for llama-mon.
//!
//! Parsed from `.env` file, CLI arguments, and environment variables.
//! Includes validation and model name filtering.

use std::path::PathBuf;
use std::time::Duration;

use clap::Parser;
use thiserror::Error;

/// Configuration for llama-mon.
///
/// Loaded from `.env` file (auto-discovered), CLI arguments, and
/// environment variables. CLI args override env vars which override defaults.
#[derive(Debug, Parser)]
#[command(name = "llama-mon", about = "llama.cpp inference server metrics monitor")]
pub struct Config {
    /// URL of the llama-server instance
    #[arg(long = "server", env = "LLAMA_MON_SERVER_URL", default_value = "http://localhost:8080")]
    pub server_url: String,

    /// Polling interval (e.g. "5s", "1m")
    #[arg(long = "poll-interval", env = "LLAMA_MON_POLL_INTERVAL", default_value = "5s")]
    pub poll_interval: humantime::Duration,

    /// Path to the SQLite database file
    #[arg(long = "db-path", env = "LLAMA_MON_DB_PATH", default_value = "~/.local/share/llama-mon/data.db")]
    pub db_path: PathBuf,

    /// HTTP request timeout (e.g. "10s", "1m")
    #[arg(long = "timeout", env = "LLAMA_MON_TIMEOUT", default_value = "10s")]
    pub timeout: humantime::Duration,

    /// Enable verbose logging
    #[arg(long = "verbose", env = "LLAMA_MON_VERBOSE")]
    pub verbose: bool,

    /// Filter by model name(s). If omitted, all models are scraped.
    /// Can be specified multiple times: --model foo --model bar
    #[arg(long = "model", short = 'm', env = "LLAMA_MON_MODEL")]
    pub models: Vec<String>,
}

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("Invalid server URL: {0}")]
    InvalidUrl(#[from] url::ParseError),
    #[error("Cannot create database directory: {0}")]
    Io(#[from] std::io::Error),
    #[error("Poll interval ({:?}) must be at least 1 second", .0)]
    PollIntervalTooShort(humantime::Duration),
    #[error("Timeout ({:?}) must be >= poll interval ({:?})", .0, .1)]
    TimeoutShorterThanPoll(humantime::Duration, humantime::Duration),
}

impl Config {
    /// Load the configuration, parsing `.env` and CLI args.
    ///
    /// `.env` is loaded silently (errors ignored) so that environment
    /// variables and CLI arguments always take precedence.
    pub fn load() -> Self {
        // Load .env file silently — failures are non-fatal
        let _ = dotenvy::dotenv();

        // Re-parse after .env to pick up any env vars set
        Self::parse()
    }

    /// Resolve the database path (expand ~) and ensure parent directory exists.
    ///
    /// Returns the resolved `url::Url` and validates:
    /// - `poll_interval >= 1s`
    /// - `timeout >= poll_interval`
    pub fn resolve(&mut self) -> Result<url::Url, ConfigError> {
        // Expand ~ in db_path
        if let Some(path) = self.db_path.to_str() {
            if let Some(stripped) = path.strip_prefix("~/") {
                if let Some(home) = dirs::home_dir() {
                    self.db_path = home.join(stripped);
                }
            }
        }

        // Create parent directory if it doesn't exist
        let parent = self.db_path.parent().expect("db_path must have a parent");
        std::fs::create_dir_all(parent)?;

        let url = url::Url::parse(&self.server_url)?;

        // Validate poll_interval >= 1s (warn if < 3s)
        let poll_dur = self.poll_duration();
        if poll_dur < Duration::from_secs(1) {
            return Err(ConfigError::PollIntervalTooShort(self.poll_interval));
        }
        if poll_dur < Duration::from_secs(3) {
            tracing::warn!(
                poll_interval = ?self.poll_interval,
                "Poll interval is very short (< 3s) — may cause high server load"
            );
        }

        // Validate timeout >= poll_interval
        let timeout_dur = self.timeout_duration();
        if timeout_dur < poll_dur {
            return Err(ConfigError::TimeoutShorterThanPoll(
                self.timeout,
                self.poll_interval,
            ));
        }

        Ok(url)
    }

    /// Whether any model name filter is active.
    pub fn has_model_filter(&self) -> bool {
        !self.models.is_empty()
    }

    /// Convert to a tokio-compatible Duration.
    pub fn poll_duration(&self) -> Duration {
        self.poll_interval.into()
    }

    /// Convert to a tokio-compatible Duration.
    pub fn timeout_duration(&self) -> Duration {
        self.timeout.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_default_values() {
        let config = Config {
            server_url: "http://localhost:8080".to_string(),
            poll_interval: humantime::Duration::from(std::time::Duration::from_secs(5)),
            db_path: PathBuf::from("~/.local/share/llama-mon/data.db"),
            timeout: humantime::Duration::from(std::time::Duration::from_secs(10)),
            verbose: false,
            models: vec![],
        };
        let url = url::Url::parse(&config.server_url).unwrap();
        assert_eq!(url.port(), Some(8080));
        assert!(!config.has_model_filter());
    }

    #[test]
    fn test_config_model_filter() {
        let config = Config {
            server_url: "http://localhost:8080".to_string(),
            poll_interval: humantime::Duration::from(std::time::Duration::from_secs(5)),
            db_path: PathBuf::from("~/.local/share/llama-mon/data.db"),
            timeout: humantime::Duration::from(std::time::Duration::from_secs(10)),
            verbose: false,
            models: vec!["foo".to_string(), "bar".to_string()],
        };
        assert!(config.has_model_filter());
        assert_eq!(config.models.len(), 2);
    }
}
