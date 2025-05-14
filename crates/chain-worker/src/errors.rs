use strata_eectl::errors::EngineError;
use strata_primitives::prelude::*;
use thiserror::Error;

/// Return type for worker messages.
pub type WorkerResult<T> = Result<T, WorkerError>;

#[derive(Debug, Error)]
pub enum WorkerError {
    #[error("missing block {0}")]
    MissingL2Block(L2BlockCommitment),

    /// This usually means that we didn't execute the previous block.
    #[error("missing pre-state to execute block {0}")]
    MissingPreState(L2BlockCommitment),

    /// This might point to a database corruption or misused admin commands.
    /// The worker should not have tried to access block outputs that are
    /// missing.
    #[error("missing exec output for block {0}")]
    MissingBlockOutput(L2BlockCommitment),

    /// This means that we haven't executed the block that's the terminal for an epoch.
    #[error("missing inner post-state of epoch {0} terminal {1}")]
    MissingInnerPostState(u64, L2BlockCommitment),

    #[error("engine: {0}")]
    Engine(#[from] EngineError),

    #[error("not yet implemented")]
    Unimplemented,
}
