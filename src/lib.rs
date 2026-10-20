//! llama-mon library — the core of the monitoring engine.
//!
//! The main binary lives in `src/main.rs`. This library exposes the
//! public API for integration tests.

pub mod config;
pub mod db;
pub mod http_client;
pub mod metrics_api;
pub mod models_api;
pub mod parse;
pub mod poller;
pub mod preset;
pub mod signals;
pub mod web;
