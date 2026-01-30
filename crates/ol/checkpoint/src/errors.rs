//! Error types for OL checkpoint builder.

use strata_db_types::errors::DbError;
use strata_primitives::{epoch::EpochCommitment, prelude::OLBlockCommitment};
use thiserror::Error;

/// Return type for worker messages.
pub type WorkerResult<T> = Result<T, OLCheckpointError>;

/// Errors that can occur during OL checkpoint operations.
#[derive(Debug, Error)]
pub enum OLCheckpointError {
    /// Missing epoch summary for the given commitment.
    #[error("missing summary for epoch commitment {0:?}")]
    MissingEpochSummary(EpochCommitment),

    /// Missing OL state snapshot for a block commitment.
    #[error("missing OL state for block commitment {0:?}")]
    MissingOLState(OLBlockCommitment),

    /// Database error.
    #[error("database failure: {0}")]
    Database(#[from] DbError),

    /// Missing a required dependency for operation.
    #[error("missing required dependency: {0}")]
    MissingDependency(&'static str),

    /// Worker has not been initialized yet.
    #[error("worker not initialized")]
    NotInitialized,

    /// Status channel failure.
    #[error("status channel failure: {0}")]
    StatusChannel(String),

    /// Functionality not yet implemented.
    #[error("not yet implemented")]
    Unimplemented,

    /// Generic unexpected error.
    #[error("unexpected failure: {0}")]
    Unexpected(String),
}
