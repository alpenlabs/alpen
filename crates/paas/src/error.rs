//! Error types for PaaS

use thiserror::Error;

/// Result type for PaaS operations
pub type PaaSResult<T> = Result<T, PaaSError>;

/// PaaS error types
#[derive(Error, Debug)]
pub enum PaaSError {
    /// Task not found
    #[error("Task not found: {0}")]
    TaskNotFound(String),

    /// Transient failure that should be retried
    #[error("Transient failure: {0}")]
    TransientFailure(String),

    /// Permanent failure that should not be retried
    #[error("Permanent failure: {0}")]
    PermanentFailure(String),

    /// Invalid state transition
    #[error("Invalid state transition from {from:?} to {to:?}")]
    InvalidTransition {
        from: crate::task::TaskStatus,
        to: crate::task::TaskStatus,
    },

    /// Worker pool error
    #[error("Worker pool error: {0}")]
    WorkerPool(String),

    /// Configuration error
    #[error("Configuration error: {0}")]
    Config(String),

    /// Internal error
    #[error("Internal error: {0}")]
    Internal(#[from] anyhow::Error),
}

impl PaaSError {
    /// Create a transient failure error
    pub fn transient(msg: impl Into<String>) -> Self {
        Self::TransientFailure(msg.into())
    }

    /// Create a permanent failure error
    pub fn permanent(msg: impl Into<String>) -> Self {
        Self::PermanentFailure(msg.into())
    }

    /// Check if this error is transient (should retry)
    pub fn is_transient(&self) -> bool {
        matches!(self, Self::TransientFailure(_))
    }

    /// Check if this error is permanent (should not retry)
    pub fn is_permanent(&self) -> bool {
        matches!(self, Self::PermanentFailure(_))
    }
}
