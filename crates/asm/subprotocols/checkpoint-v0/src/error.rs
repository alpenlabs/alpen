//! Error types for checkpoint v0 subprotocol

use strata_asm_proto_checkpoint_txs::CheckpointTxError;
use strata_identifiers::Epoch;
use thiserror::Error;

/// Errors that can occur during checkpoint verification and processing
#[derive(Debug, Error)]
pub enum CheckpointV0Error {
    /// Checkpoint parsing failed
    #[error("Failed to parse checkpoint: {0}")]
    ParsingError(String),

    /// Signature verification failed
    #[error("Checkpoint signature verification failed")]
    InvalidSignature,

    /// Checkpoint Proof verification failed
    #[error("Checkpoint proof verification failed")]
    InvalidCheckpointProof,

    /// Invalid epoch progression
    #[error("Invalid epoch: expected {expected}, got {actual}")]
    InvalidEpoch { expected: Epoch, actual: Epoch },

    /// Serialization error
    #[error("Serialization error")]
    SerializationError,

    /// Invalid transaction type
    #[error("Unsupported transaction type: {0}")]
    UnsupportedTxType(u8),

    /// Batch info epoch does not align with transition epoch
    #[error(
        "Checkpoint batch info epoch {info_epoch} differs from transition epoch {transition_epoch}"
    )]
    BatchEpochMismatch {
        info_epoch: Epoch,
        transition_epoch: Epoch,
    },

    /// State roots between consecutive checkpoints do not align
    #[error("Checkpoint state root mismatch between consecutive epochs")]
    StateRootMismatch,
}

/// Result type alias for checkpoint operations
pub type CheckpointV0Result<T> = Result<T, CheckpointV0Error>;

impl From<CheckpointTxError> for CheckpointV0Error {
    fn from(err: CheckpointTxError) -> Self {
        match err {
            CheckpointTxError::UnexpectedTxType {
                expected: _,
                actual,
            } => CheckpointV0Error::UnsupportedTxType(actual),
            other => CheckpointV0Error::ParsingError(other.to_string()),
        }
    }
}
