//! Error types for the CSM worker.

use strata_asm_common::{AsmError, AuxError};
use strata_asm_manifest_types::AsmManifestError;
use strata_checkpoint_verification::CheckpointValidationError;
use strata_db_types::errors::DbError;
use strata_primitives::l1::{L1BlockCommitment, L1BlockId};
use thiserror::Error;

/// Return type for CSM worker operations.
pub type CsmWorkerResult<T> = Result<T, CsmWorkerError>;

/// Errors that can occur while the CSM worker processes ASM status updates.
#[derive(Debug, Error)]
pub enum CsmWorkerError {
    /// No committed ASM block exists yet; the worker was bootstrapped without one.
    #[error("CSM has no last committed anchor ASM block")]
    NoAnchorAsmBlock,

    /// The genesis L1 block has no parent to derive a commitment from.
    #[error("cannot derive parent for genesis L1 block {0}")]
    GenesisHasNoParent(L1BlockCommitment),

    /// No checkpoint envelope tx in the block validated for the logged tip.
    #[error("no checkpoint envelope tx in L1 block {asm_block} validated for epoch {epoch}")]
    NoMatchingCheckpoint {
        asm_block: L1BlockCommitment,
        epoch: u32,
    },

    /// The checkpoint subprotocol section was absent from the ASM state.
    #[error("checkpoint subprotocol section missing in ASM state")]
    MissingCheckpointSection,

    /// Failed to deserialize a `CheckpointTipUpdate` log entry.
    #[error("failed to deserialize CheckpointTipUpdate log: {0}")]
    DeserializeTipLog(#[from] AsmManifestError),

    /// Failed to decode the checkpoint subprotocol's typed state.
    #[error("decode checkpoint subprotocol state: {0}")]
    DecodeCheckpointSection(#[source] AsmError),

    /// Verifying ASM aux data against the parent accumulator failed.
    #[error("verify ASM aux data: {0}")]
    VerifyAuxData(#[from] AuxError),

    /// A candidate checkpoint failed validation.
    #[error("checkpoint validation failed: {0}")]
    CheckpointValidation(#[from] CheckpointValidationError),

    /// A storage operation failed.
    #[error("database failure: {0}")]
    Database(#[from] DbError),

    /// Fetching an L1 block from bitcoind failed. The cause is kept as a
    /// `String` so the worker does not depend on the RPC client's error type.
    #[error("fetch L1 block {blockid}: {cause}")]
    L1Fetch { blockid: L1BlockId, cause: String },

    /// A record the worker expected was absent.
    #[error("missing {what}: {detail}")]
    MissingData { what: &'static str, detail: String },

    /// A reorg diverged at or below the finalized anchor — a protocol violation.
    #[error("reorg past finality: finalized {finalized}, incoming {incoming}")]
    ReorgPastFinality {
        finalized: L1BlockCommitment,
        incoming: L1BlockCommitment,
    },

    /// Other generic errors without precise types.
    #[error("{0}")]
    Context(String),
}
