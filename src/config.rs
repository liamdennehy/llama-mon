//! Configuration for llama-mon.
//!
//! Parsed from CLI arguments and environment variables.

use std::path::PathBuf;
use std::time::Duration;

use clap::Parser;
use thiserror::Error;

#[derive(Debug, Parser)]
#[command(name = "llama-mon", about = "llama.cpp inference server metrics monitor")]
pub struct Config {
    /// URL of the llama-server instance
    #[arg(env = "LLAMA_MON_SERVER_URL", default_value = "http://localhost:8080")]
    pub server_url: String,

    /// Polling interval (e.g. "5s", "1m")
    #[arg(env = "LLAMA_MON_POLL_INTERVAL", default_value = "5s")]
    pub poll_interval: humantime::Duration,

    /// Path to the SQLite database file
    #[arg(env = "LLAMA_MON_DB_PATH", default_value = "~/.local/share/llama-mon/data.db")]
    pub db_path: PathBuf,

    /// HTTP request timeout (e.g. "10s", "1m")
    #[arg(env = "LLAMA_MON_TIMEOUT", default_value = "10s")]
    pub timeout: humantime::Duration,

    /// Enable verbose logging
    #[arg(env = "LLAMA_MON_VERBOSE", long)]
    pub verbose: bool,
}

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("Invalid server URL: {0}")]
    InvalidUrl(#[from] url::ParseError),
    #[error("Cannot create database directory: {0}")]
    Io(#[from] std::io::Error),
}

impl Config {
    /// Resolve the database path (expand ~) and ensure parent directory exists.
    pub fn resolve(&mut self) -> Result<url::Url, ConfigError> {
        // Expand ~ in db_path
        if let Some(path) = self.db_path.to_str() {
            if let Some(stripped) = path.strip_prefix("~/") {
                if let Some(home) = dirs::home_dir() {
                    self.db_path = home.join(stripped);
                }
            }
        }

        // Create parent directory
        std::fs::create_dir_all(self.db_path.parent().expect("db_path must have a parent"))?;

        let url = url::Url::parse(&self.server_url)?;
        Ok(url)
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
        };
        let url = url::Url::parse(&config.server_url).unwrap();
        assert_eq!(url.port(), Some(8080));
    }
}
