//! Error types for checkpointing v0 subprotocol

use strata_asm_proto_checkpointing_txs::CheckpointTxError;
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
    InvalidEpoch { expected: u64, actual: u64 },

    /// Serialization error
    #[error("Serialization error")]
    SerializationError,

    /// Invalid transaction type
    #[error("Unsupported transaction type: {0}")]
    UnsupportedTxType(String),

    /// State transition validation failed
    #[error("State transition validation failed: {0}")]
    StateTransitionError(String),
}

/// Result type alias for checkpoint operations
pub type CheckpointV0Result<T> = Result<T, CheckpointV0Error>;

impl From<CheckpointTxError> for CheckpointV0Error {
    fn from(err: CheckpointTxError) -> Self {
        match err {
            CheckpointTxError::UnexpectedTxType { expected, actual } => {
                CheckpointV0Error::UnsupportedTxType(format!(
                    "Expected checkpoint tx type {expected}, got {actual}"
                ))
            }
            other => CheckpointV0Error::ParsingError(other.to_string()),
        }
    }
}
