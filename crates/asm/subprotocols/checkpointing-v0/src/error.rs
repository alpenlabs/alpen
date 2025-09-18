//! Error types for checkpointing v0 subprotocol

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

    /// Proof verification failed
    #[error("Checkpoint proof verification failed")]
    InvalidProof,

    /// Invalid epoch progression
    #[error("Invalid epoch: expected {expected}, got {actual}")]
    InvalidEpoch { expected: u64, actual: u64 },

    /// Invalid L1 view height mismatch
    #[error("L1 view height mismatch: expected {expected}, got {actual}")]
    L1ViewMismatch { expected: u64, actual: u64 },

    /// Serialization error
    #[error("Serialization error")]
    SerializationError,

    /// Invalid transaction type
    #[error("Unsupported transaction type: {0}")]
    UnsupportedTxType(String),

    /// State transition validation failed
    #[error("State transition validation failed: {0}")]
    StateTransitionError(String),

    /// Invalid verifying key format
    #[error("Invalid verifying key format: {0}")]
    InvalidVerifyingKeyFormat(String),

    /// L1 to L2 message validation failed
    #[error("L1 to L2 message validation failed: {0}")]
    L1ToL2MessageError(String),

    /// L2 to L1 message validation failed
    #[error("L2 to L1 message validation failed: {0}")]
    L2ToL1MessageError(String),

    /// Auxiliary data error
    #[error("Auxiliary data error: {0}")]
    AuxDataError(String),
}

/// Result type alias for checkpoint operations
pub type CheckpointV0Result<T> = Result<T, CheckpointV0Error>;