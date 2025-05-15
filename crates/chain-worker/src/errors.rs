use strata_eectl::errors::EngineError;
use strata_primitives::prelude::*;
use thiserror::Error;

/// Return type for worker messages.
pub type WorkerResult<T> = Result<T, WorkerError>;

#[derive(Debug, Error)]
pub enum WorkerError {
    #[error("missing block {0}")]
    MissingL2Block(L2BlockId),

    /// This usually means that we didn't execute the previous block.
    #[error("missing pre-state to execute block {0:?}")]
    MissingPreState(L2BlockCommitment),

    /// This might point to a database corruption or misused admin commands.
    /// The worker should not have tried to access block outputs that are
    /// missing.
    #[error("missing exec output for block {0:?}")]
    MissingBlockOutput(L2BlockCommitment),

    /// This means that we haven't executed the block that's the terminal for an epoch.
    #[error("missing inner post-state of epoch {0} terminal {1:?}")]
    MissingInnerPostState(u64, L2BlockCommitment),

    #[error("OL block execution: {0}")]
    Exec(#[from] strata_chainexec::Error),

    #[error("engine: {0}")]
    Engine(#[from] EngineError),

    #[error("not yet implemented")]
    Unimplemented,
}

impl Into<strata_chainexec::Error> for WorkerError {
    fn into(self) -> strata_chainexec::Error {
        // TODO implement the rest of these, somehow
        match self {
            WorkerError::MissingL2Block(l2_block_id) => {
                strata_chainexec::Error::MissingL2Header(l2_block_id)
            }
            WorkerError::MissingPreState(l2_block_commitment) => todo!(),
            WorkerError::MissingBlockOutput(l2_block_commitment) => todo!(),
            WorkerError::MissingInnerPostState(_, l2_block_commitment) => todo!(),
            WorkerError::Exec(error) => todo!(),
            WorkerError::Engine(engine_error) => todo!(),
            WorkerError::Unimplemented => strata_chainexec::Error::Unimplemented,
        }
    }
}
