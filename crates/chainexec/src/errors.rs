use strata_primitives::prelude::*;
use thiserror::Error;

/// Newtype for exec context results, for brevity.
pub type ExecResult<T> = Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("missing pre-state for block {0}")]
    MissingBlockPreState(L2BlockId),

    #[error("missing post-state for block {0}")]
    MissingBlockPostState(L2BlockId),

    #[error("missing L2 block header {0}")]
    MissingL2Header(L2BlockId),

    #[error("computed state root mismatch with block state root")]
    StateRootMismatch,

    #[error("transition: {0}")]
    Transition(#[from] strata_chaintsn::errors::TsnError),

    #[error("not yet implemented")]
    Unimplemented,

    /// Some unexpected error condition happened.
    ///
    /// This only exists as a way to map worker error types that we don't expect
    /// to be generated in calls that the executor would see, like
    /// `WorkerExited` or `Exec`.
    #[error("unexpected failure: {0}")]
    Unexpected(String),
}
