//! Error types for PaaS

use strata_service::ServiceError;
use thiserror::Error;
use uuid::Uuid;

use crate::TaskId;

/// Error type for PaaS operations
#[derive(Debug, Error)]
pub enum PaaSError {
    /// Missing required dependency during initialization
    #[error("missing dependency: {0}")]
    MissingDependency(&'static str),

    /// Task not found
    #[error("task not found: {0}")]
    TaskNotFound(TaskId),

    /// Task already exists
    #[error("task already exists: {0}")]
    TaskAlreadyExists(TaskId),

    /// Invalid proof context
    #[error("invalid context: {0}")]
    InvalidContext(String),

    /// Storage error
    #[error("storage error: {0}")]
    Storage(String),

    /// Proving task error (from prover-client)
    #[error("proving error: {0}")]
    Proving(String),

    /// No workers available
    #[error("worker unavailable")]
    WorkerUnavailable,

    /// Service framework error
    #[error("service error: {0}")]
    Service(ServiceError),

    /// Failed to launch service
    #[error("launch failed: {0}")]
    LaunchFailed(String),

    /// Worker exited unexpectedly
    #[error("worker exited")]
    WorkerExited,

    /// Operation cancelled
    #[error("operation cancelled")]
    Cancelled,

    /// Generic unexpected error
    #[error("unexpected error: {0}")]
    Unexpected(String),
}

/// Convert ServiceError to PaaSError
impl From<ServiceError> for PaaSError {
    fn from(err: ServiceError) -> Self {
        match err {
            ServiceError::WorkerExited | ServiceError::WorkerExitedWithoutResponse => {
                PaaSError::WorkerExited
            }
            ServiceError::WaitCancelled => PaaSError::Cancelled,
            ServiceError::BlockingThreadPanic(msg) => {
                PaaSError::Unexpected(format!("blocking thread panicked: {}", msg))
            }
            ServiceError::UnknownInputErr => PaaSError::Unexpected("unknown input error".to_string()),
        }
    }
}

/// Result type for PaaS operations
pub type PaaSResult<T> = Result<T, PaaSError>;
