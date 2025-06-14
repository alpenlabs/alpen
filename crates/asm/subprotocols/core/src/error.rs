//! Error types for the OL Core subprotocol

use thiserror::Error;

/// Result type alias for the OL Core subprotocol
pub type Result<T> = std::result::Result<T, CoreError>;

// TODO: Review and refine error variants as needed
/// Errors that can occur in the OL Core subprotocol
#[derive(Debug, Error)]
pub enum CoreError {
    /// Invalid signature on checkpoint
    #[error("Invalid signature on checkpoint")]
    InvalidSignature,

    /// Invalid epoch number
    #[error("Invalid epoch number in checkpoint: expected {expected}, got {actual}")]
    InvalidEpoch { expected: u32, actual: u32 },

    /// Invalid L2 Block slot
    #[error(
        "Invalid L2 block slot: new slot {new_slot} must be greater than previous slot {prev_slot}"
    )]
    InvalidL2BlockSlot { prev_slot: u64, new_slot: u64 },

    /// Invalid L1 Block height
    #[error("Invalid L1 block height: {reason}")]
    InvalidL1BlockHeight { reason: String },

    /// Missing required field in L2 to L1 message
    #[error("Missing required field '{field}' in L2 to L1 message at index {index}")]
    MissingRequiredFieldInL2ToL1Msg { index: usize, field: String },

    /// L1 to L2 message range mismatch
    #[error("L1 to L2 message range commitment does not match")]
    L1ToL2RangeMismatch,

    /// Malformed signed checkpoint
    #[error("Failed to extract signed checkpoint from transaction: {reason}")]
    MalformedSignedCheckpoint { reason: String },

    /// Serialization error
    #[error("Failed to serialize data")]
    SerializationError,

    /// Transaction parsing error
    #[error("Failed to parse transaction data: {0}")]
    TxParsingError(String),

    /// Invalid ZK proof
    #[error("Invalid ZK Proof")]
    InvalidProof,

    /// Invalid verifying key format
    #[error("Invalid verifying key format: {0}")]
    InvalidVerifyingKeyFormat(String),
}
