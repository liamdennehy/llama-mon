//! Configuration for llama-mon.
//!
//! Parsed from `.env` file, CLI arguments, and environment variables.
//! Includes validation and multi-server support.

use std::path::PathBuf;
use std::time::Duration;

use clap::Parser;
use thiserror::Error;

/// A single monitored server (URL + active flag).
#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub url: url::Url,
    pub active: bool,
}

impl ServerConfig {
    /// Parse a server from a "host:port" string.
    pub fn from_host_port(host_port: &str, active: bool) -> Result<Self, ConfigError> {
        // Try to parse as a full URL first
        let url = if host_port.contains("://") {
            url::Url::parse(host_port)?
        } else {
            // Default to http:// for bare "host:port"
            url::Url::parse(&format!("http://{}", host_port))?
        };
        Ok(Self { url, active })
    }

    /// Human-readable identifier (host:port).
    pub fn identifier(&self) -> String {
        format!(
            "{}:{}",
            self.url.host_str().unwrap_or("unknown"),
            self.url.port().unwrap_or(0)
        )
    }
}

/// A list of servers to monitor.
pub type ServerList = Vec<ServerConfig>;

/// Configuration for llama-mon.
///
/// Loaded from `.env` file (auto-discovered), CLI arguments, and
/// environment variables. CLI args override env vars which override defaults.
#[derive(Debug, Parser)]
#[command(name = "llama-mon", about = "llama.cpp inference server metrics monitor", version)]
#[derive(Clone)]
pub struct Config {
    /// URL or host:port of a llama-server instance to monitor.
    /// Can be specified multiple times. Defaults to `http://localhost:8080`
    /// if no --server flag is provided (for backward compatibility).
    #[arg(long = "server", env = "LLAMA_MON_SERVER_URL")]
    pub server_urls: Vec<String>,

    /// Parsed server configurations (populated by `load()`).
    #[arg(skip)]
    pub servers: Vec<ServerConfig>,

    /// Polling interval (e.g. "5s", "1m")
    #[arg(long = "poll-interval", env = "LLAMA_MON_POLL_INTERVAL", default_value = "5s")]
    pub poll_interval: humantime::Duration,

    /// Full path to the SQLite database file (parent directory is created if needed).
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

    /// Exit after the first unsuccessful poll cycle instead of looping.
    /// Useful for debugging transient errors.
    #[arg(long = "fail-fast")]
    pub fail_fast: bool,

    /// IP address for the embedded web dashboard to listen on.
    /// Defaults to `127.0.0.1` (localhost only). Set to `0.0.0.0` for
    /// all interfaces.
    #[arg(long = "listen-ip", env = "LLAMA_MON_LISTEN_IP", default_value = "127.0.0.1")]
    pub listen_ip: String,

    /// Port for the embedded web dashboard.
    /// If omitted (and not set via `LLAMA_MON_WEB_PORT`), the web server
    /// is disabled. Set to `0` to disable explicitly.
    #[arg(long = "listen-port", env = "LLAMA_MON_WEB_PORT")]
    pub web_port: Option<u16>,
}

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("Invalid server URL: {0}")]
    InvalidUrl(#[from] url::ParseError),
    #[error("Cannot create database directory: {0}")]
    Io(#[from] std::io::Error),
    #[error("Poll interval ({:?}) must be at least 1 second", .0)]
    PollIntervalTooShort(humantime::Duration),
    #[error("At least one mode must be enabled: provide --server (or LLAMA_MON_SERVER_URL) for scraper mode, or --port (or LLAMA_MON_WEB_PORT) for web dashboard mode")]
    NoModeEnabled,
}

impl Config {
    /// Load the configuration, parsing `.env` and CLI args.
    ///
    /// `.env` is loaded silently (errors ignored) so that environment
    /// variables and CLI arguments always take precedence.
    pub fn load() -> Self {
        // Load .env file silently — failures are non-fatal
        let _ = dotenvy::dotenv();

        let config = Self::parse();

        // Parse raw URL strings into ServerConfig structs.
        // Backwards compat: if no --server flags were given, use the
        // LLAMA_MON_SERVER_URL env var as a default single server.
        let urls = if config.server_urls.is_empty() {
            vec![std::env::var("LLAMA_MON_SERVER_URL")
                .unwrap_or_else(|_| "http://localhost:8080".to_string())]
        } else {
            config.server_urls.clone()
        };

        let servers = urls
            .into_iter()
            .map(|u| ServerConfig::from_host_port(&u, true))
            .collect::<Result<Vec<_>, _>>()
            .expect("invalid server URL in config");

        Self {
            server_urls: config.server_urls,
            servers,
            poll_interval: config.poll_interval,
            db_path: config.db_path,
            timeout: config.timeout,
            verbose: config.verbose,
            models: config.models,
            fail_fast: config.fail_fast,
            listen_ip: config.listen_ip,
            web_port: config.web_port,
        }
    }

    /// Resolve the database path (expand ~) and ensure parent directory exists.
    ///
    /// Validates:
    /// - `poll_interval >= 1s`
    pub fn resolve(&mut self) -> Result<(), ConfigError> {
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

        // Validate: at least one mode must be enabled
        let has_scraper = !self.servers.is_empty();
        let has_web = self.web_port.unwrap_or(0) > 0;
        if !has_scraper && !has_web {
            return Err(ConfigError::NoModeEnabled);
        }

        Ok(())
    }

    /// Return the list of active servers to poll.
    pub fn active_servers(&self) -> Vec<&ServerConfig> {
        self.servers.iter().filter(|s| s.active).collect()
    }

    /// Whether any server is configured.
    pub fn has_servers(&self) -> bool {
        !self.servers.is_empty()
    }

    /// Whether any model name filter is active.
    pub fn has_model_filter(&self) -> bool {
        !self.models.is_empty()
    }

    /// Whether all models should be scraped (no filter or explicit "all").
    pub fn is_all_models(&self) -> bool {
        self.models.is_empty() || self.models.iter().any(|m| m == "all")
    }

    /// Check if a model name matches the current filter.
    /// Returns `true` if no filter is active (all models match).
    pub fn model_matches(&self, model_name: &str) -> bool {
        if self.is_all_models() {
            return true;
        }
        self.models.iter().any(|m| m.eq_ignore_ascii_case(model_name))
    }

    /// Whether the scraper (poller) should run.
    /// Returns `true` if any server URLs are configured.
    pub fn should_scrub(&self) -> bool {
        !self.servers.is_empty()
    }

    /// Whether the web dashboard server should run.
    /// Returns `true` if `--port` is set to a value > 0.
    pub fn should_serve_web(&self) -> bool {
        self.web_port.unwrap_or(0) > 0
    }

    /// Effective web port (0 if web server is disabled).
    pub fn web_port_effective(&self) -> u16 {
        self.web_port.unwrap_or(0)
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

    fn make_config(servers: Vec<ServerConfig>) -> Config {
        let server_urls: Vec<String> = servers.iter().map(|s| s.identifier()).collect();
        Config {
            server_urls,
            servers,
            poll_interval: humantime::Duration::from(std::time::Duration::from_secs(5)),
            db_path: PathBuf::from("/tmp/llama-mon-test.db"),
            timeout: humantime::Duration::from(std::time::Duration::from_secs(10)),
            verbose: false,
            models: vec![],
            fail_fast: false,
            listen_ip: "127.0.0.1".to_string(),
            web_port: Some(8081),
        }
    }

    fn make_config_web_disabled(servers: Vec<ServerConfig>) -> Config {
        let server_urls: Vec<String> = servers.iter().map(|s| s.identifier()).collect();
        Config {
            server_urls,
            servers,
            poll_interval: humantime::Duration::from(std::time::Duration::from_secs(5)),
            db_path: PathBuf::from("/tmp/llama-mon-test.db"),
            timeout: humantime::Duration::from(std::time::Duration::from_secs(10)),
            verbose: false,
            models: vec![],
            fail_fast: false,
            listen_ip: "127.0.0.1".to_string(),
            web_port: None,
        }
    }

    #[test]
    fn test_config_default_values() {
        let config = make_config(vec![ServerConfig {
            url: url::Url::parse("http://localhost:8080").unwrap(),
            active: true,
        }]);
        assert_eq!(config.servers.len(), 1);
        assert_eq!(config.servers[0].url.port(), Some(8080));
        assert!(!config.has_model_filter());
        assert!(config.should_scrub());
        assert!(config.should_serve_web());
        assert_eq!(config.web_port_effective(), 8081);
    }

    #[test]
    fn test_config_multi_server() {
        let config = make_config(vec![
            ServerConfig {
                url: url::Url::parse("http://server1:8080").unwrap(),
                active: true,
            },
            ServerConfig {
                url: url::Url::parse("http://server2:8080").unwrap(),
                active: false,
            },
        ]);
        assert_eq!(config.servers.len(), 2);
        assert_eq!(config.active_servers().len(), 1);
        assert_eq!(config.models.len(), 0);
    }

    #[test]
    fn test_config_model_filter() {
        let mut config = make_config(vec![ServerConfig {
            url: url::Url::parse("http://localhost:8080").unwrap(),
            active: true,
        }]);
        config.models = vec!["foo".to_string(), "bar".to_string()];
        assert!(config.has_model_filter());
        assert_eq!(config.models.len(), 2);
        assert!(config.model_matches("foo"));
        assert!(!config.model_matches("baz"));
    }

    #[test]
    fn test_config_opt_out_flags() {
        let config = make_config_web_disabled(vec![ServerConfig {
            url: url::Url::parse("http://localhost:8080").unwrap(),
            active: true,
        }]);
        assert!(config.should_scrub());
        assert!(!config.should_serve_web());
        assert_eq!(config.web_port_effective(), 0);
    }

    #[test]
    fn test_server_config_from_host_port() {
        let s = ServerConfig::from_host_port("localhost:8080", true).unwrap();
        assert_eq!(s.url.port(), Some(8080));
        assert_eq!(s.url.host_str(), Some("localhost"));
        assert_eq!(s.identifier(), "localhost:8080");
        assert!(s.active);

        let s = ServerConfig::from_host_port("http://10.0.0.1:9090", false).unwrap();
        assert_eq!(s.url.port(), Some(9090));
        assert!(!s.active);

        let s = ServerConfig::from_host_port("localhost:42", false).unwrap();
        assert!(!s.active);
        assert_eq!(s.identifier(), "localhost:42");
    }
}
