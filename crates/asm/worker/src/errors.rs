use strata_btc_types::BitcoinTxid;
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

    #[error("not yet implemented")]
    Unimplemented,

    // Auxiliary data resolution errors
    #[error("Bitcoin transaction not found: {0:?}")]
    BitcoinTxNotFound(BitcoinTxid),

    #[error("L1 block not found at height {height}")]
    L1BlockNotFound { height: u64 },

    #[error("No ASM state available")]
    NoAsmState,

    #[error("Invalid manifest hash range: start={start}, end={end}")]
    InvalidManifestRange { start: u64, end: u64 },

    #[error("Invalid L1 height range: start={start}, end={end}")]
    InvalidHeightRange { start: u64, end: u64 },

    #[error("Manifest hash not found for MMR index {index}")]
    ManifestHashNotFound { index: u64 },

    #[error("MMR proof generation failed for index {index}")]
    MmrProofFailed { index: u64 },
}
