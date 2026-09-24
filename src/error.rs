use thiserror::Error;

#[derive(Debug, Error)]
pub enum CompassError {
    #[error("sqlite error: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("xml parse error at byte {pos}: {msg}")]
    Xml { pos: u64, msg: String },
    #[error("serde error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("conflict: branch head moved (expected {expected}, found {actual})")]
    Concurrency { expected: String, actual: String },
    #[error("not found: {0}")]
    NotFound(String),
    #[error("bad request: {0}")]
    Bad(String),
}

pub type Result<T> = std::result::Result<T, CompassError>;
