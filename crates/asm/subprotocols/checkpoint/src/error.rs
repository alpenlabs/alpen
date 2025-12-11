//! Error types for the checkpoint subprotocol.

use strata_asm_proto_checkpoint_txs::CheckpointTxError;
use strata_identifiers::{Epoch, Slot};
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

    /// L1 height did not progress correctly.
    #[error("invalid L1 height progression: previous {previous}, new {new}")]
    InvalidL1Height { previous: u64, new: u64 },

    /// L2 slot did not progress correctly.
    #[error("invalid L2 slot progression: previous {previous}, new {new}")]
    InvalidL2Slot { previous: Slot, new: Slot },

    /// Failed to verify checkpoint proof.
    #[error("proof verification failed")]
    ProofVerification,

    /// Missing auxiliary data for manifest hashes.
    #[error("missing manifest hash auxiliary data")]
    MissingManifestHashes,
}
