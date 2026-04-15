//! Error types.
//!
//! Variants are typed (no erased `anyhow::Error` wrapped inside the enum).
//! Wrapping `anyhow` inside a `thiserror` enum muddies downcast behavior
//! for callers — keep the library boundary typed and let applications
//! decide whether to erase upstream.

pub type ProverResult<T> = Result<T, ProverError>;

#[derive(Debug, thiserror::Error)]
pub enum ProverError {
    #[error("task not found: {0}")]
    TaskNotFound(String),

    #[error("task already exists: {0}")]
    TaskAlreadyExists(String),

    #[error("no receipt store configured")]
    NoReceiptStore,

    #[error("transient: {0}")]
    TransientFailure(String),

    #[error("permanent: {0}")]
    PermanentFailure(String),

    /// Backend IO failure (sled, filesystem, tokio runtime, ...).
    #[error("storage: {0}")]
    Storage(String),

    /// Encode or decode of a stored record failed.
    #[error("codec: {0}")]
    Codec(String),

    /// Command channel failure (send/recv/cancelled).
    #[error("command channel: {0}")]
    Command(String),
}

impl ProverError {
    pub fn is_transient(&self) -> bool {
        matches!(self, Self::TransientFailure(_))
    }

    pub fn is_permanent(&self) -> bool {
        matches!(self, Self::PermanentFailure(_))
    }
}
