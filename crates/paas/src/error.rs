//! Error types for PaaS

use thiserror::Error;

/// Result type for PaaS operations
pub type ProverServiceResult<T> = Result<T, ProverServiceError>;

/// PaaS error types
#[derive(Error, Debug)]
pub enum ProverServiceError {
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

impl ProverServiceError {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = ProverServiceError::TaskNotFound("task_123".into());
        assert!(err.to_string().contains("task_123"));

        let err = ProverServiceError::TransientFailure("network error".into());
        assert!(err.to_string().contains("network error"));

        let err = ProverServiceError::PermanentFailure("invalid input".into());
        assert!(err.to_string().contains("invalid input"));

        let err = ProverServiceError::Config("missing config".into());
        assert!(err.to_string().contains("missing config"));
    }

    #[test]
    fn test_error_helpers() {
        let err = ProverServiceError::transient("test");
        assert!(err.is_transient());
        assert!(!err.is_permanent());

        let err = ProverServiceError::permanent("test");
        assert!(err.is_permanent());
        assert!(!err.is_transient());
    }
}
