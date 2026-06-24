use thiserror::Error;

/// Errors surfaced by the core layer.
#[derive(Debug, Error)]
pub enum CoreError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON (de)serialization error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("profile not found: {0}")]
    ProfileNotFound(String),

    #[error("invalid domain: {0}")]
    InvalidDomain(String),

    #[error("no working strategy found")]
    NoStrategyFound,

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, CoreError>;
