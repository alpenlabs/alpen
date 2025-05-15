use strata_primitives::prelude::*;
use thiserror::Error;

/// Newtype for exec context results, for brevity.
pub type ExecResult<T> = Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("missing post-state for block {0}")]
    MissingBlockPostState(L2BlockId),

    #[error("missing L2 block header {0}")]
    MissingL2Header(L2BlockId),

    #[error("transition: {0}")]
    Transition(#[from] strata_chaintsn::errors::TsnError),

    #[error("computed state root mismatch with block state root")]
    StateRootMismatch,

    #[error("not yet implemented")]
    Unimplemented,
}
