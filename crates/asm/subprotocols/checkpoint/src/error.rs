//! Error types for the checkpoint subprotocol.

use strata_asm_common::AuxError;
use strata_asm_manifest_types::AsmManifestError;
use strata_asm_proto_checkpoint_txs::CheckpointTxError;
use strata_identifiers::{Epoch, L1BlockId, OLBlockId};
use strata_predicate::PredicateError;
use thiserror::Error;

/// Result type for checkpoint subprotocol operations.
pub(crate) type CheckpointResult<T> = Result<T, CheckpointError>;

/// Errors that can occur during checkpoint processing.
#[derive(Debug, Error)]
pub(crate) enum CheckpointError {
    /// Failed to parse checkpoint transaction.
    #[error("checkpoint parsing error: {0}")]
    Parsing(#[from] CheckpointTxError),

    /// Checkpoint signature verification failed.
    #[error("invalid checkpoint signature")]
    InvalidSignature,

    /// Checkpoint epoch is not sequential.
    #[error("invalid epoch: expected {expected}, got {actual}")]
    InvalidEpoch { expected: Epoch, actual: Epoch },

    /// L1 start did not match previous checkpoint commitment.
    #[error("L1 start commitment mismatch")]
    InvalidL1Start {
        expected_height: u32,
        expected_blkid: L1BlockId,
        new_height: u32,
        new_blkid: L1BlockId,
    },

    /// L2 start did not match previous checkpoint terminal commitment.
    #[error("L2 start commitment mismatch")]
    InvalidL2Start {
        expected_slot: u64,
        expected_blkid: OLBlockId,
        new_slot: u64,
        new_blkid: OLBlockId,
    },

    /// L1 height did not progress correctly.
    #[error("L1 height did not progress: previous {previous}, new {new}")]
    InvalidL1Progression { previous: u32, new: u32 },

    /// L2 slot did not progress correctly.
    #[error("L2 slot did not progress: previous {previous}, new {new}")]
    InvalidL2Progression { previous: u64, new: u64 },

    /// Failed to retrieve manifest hashes from auxiliary data.
    #[error("auxiliary data error: {0}")]
    AuxData(#[from] AuxError),

    /// Checkpoint must cover at least one L1 block.
    #[error("checkpoint must cover at least one L1 block (empty manifest hashes)")]
    EmptyManifestHashes,

    /// Checkpoint proof verification failed.
    #[error("proof verification failed: {0}")]
    ProofVerification(#[from] PredicateError),

    /// Failed to create checkpoint update log entry.
    #[error("log emission failed: {0}")]
    LogEmission(#[from] AsmManifestError),
}
