//! Error types for checkpoint operations.

use thiserror::Error;

/// Errors that can occur during checkpoint operations.
#[derive(Debug, Error)]
pub enum CheckpointError {
    /// Failed to serialize data.
    #[error("serialization error: {0}")]
    Serialization(String),

    /// Failed to deserialize data.
    #[error("deserialization error: {0}")]
    Deserialization(String),

    /// Invalid checkpoint data.
    #[error("invalid checkpoint: {0}")]
    InvalidCheckpoint(String),
}

/// Result type for checkpoint operations.
pub type CheckpointResult<T> = Result<T, CheckpointError>;
