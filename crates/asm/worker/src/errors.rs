use strata_primitives::prelude::*;
use thiserror::Error;

/// Return type for worker messages.
pub type WorkerResult<T> = Result<T, WorkerError>;

#[derive(Debug, Error)]
pub enum WorkerError {
    #[error("ASM error: {0}")]
    AsmError(#[from] strata_asm_common::AsmError),

    #[error("missing genesis ASM state.")]
    MissingGenesisState,

    #[error("missing l1 block {0:?}")]
    MissingL1Block(L1BlockId),

    #[error("missing ASM state for the block {0:?}")]
    MissingAsmState(L1BlockId),

    #[error("btc client error")]
    BtcClient,

    #[error("db error")]
    DbError,

    #[error("missing required dependency: {0}")]
    MissingDependency(&'static str),
    #[error("unexpected error: {0}")]
    Unexpected(String),

    #[error("not yet implemented")]
    Unimplemented,
}
