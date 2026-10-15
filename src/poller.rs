//! Placeholder — main polling loop lives here.

use thiserror::Error;

#[derive(Error, Debug)]
pub enum PollerError {
    #[error("HTTP error: {0}")]
    Http(#[from] crate::http_client::HttpError),
    #[error("Database error: {0}")]
    Db(#[from] crate::db::DbError),
    #[error("Poller stopped")]
    Stopped,
}
