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
    #[error("Invalid epoch number in checkpoint")]
    InvalidEpoch,

    /// Invalid L2 Block slot
    #[error("Invalid l2 block slot in checkpoint")]
    InvalidL2BlockSlot,

    /// Invalid L1 Block hight
    #[error("Invalid l1 block hight")]
    InvalidL1BlockHeight,

    /// Missing required field in L2 to L1 message
    #[error("Missing required field '{field}' in L2 to L1 message at index {index}")]
    MissingRequiredFieldInL2ToL1Msg { index: usize, field: String },

    /// State diff hash mismatch
    #[error("State diff hash does not match the one in public parameters")]
    StateDiffMismatch,

    /// Unexpected previous L2 terminal
    #[error("Previous L2 terminal does not match expected value")]
    UnexpectedPrevTerminal,

    /// Unexpected previous L1 reference
    #[error("Previous L1 reference does not match expected value")]
    UnexpectedPrevL1Ref,

    /// L1 to L2 message range mismatch
    #[error("L1 to L2 message range commitment does not match")]
    L1ToL2RangeMismatch,

    /// Proof verification failed
    #[error("ZK-SNARK proof verification failed")]
    ProofVerificationFailed,

    /// Malformed signed checkpoint
    #[error("Failed to extract signed checkpoint from transaction")]
    MalformedSignedCheckpoint,

    /// Malformed public parameters
    #[error("Failed to deserialize public parameters from proof")]
    MalformedPublicParams,

    /// Serialization error
    #[error("Failed to serialize data")]
    SerializationError,

    /// Transaction parsing error
    #[error("Failed to parse transaction data: {0}")]
    TxParsingError(String),

    /// Invalid ZK proof
    #[error("Invalid ZK Proof")]
    InvalidProof,
}

impl From<borsh::io::Error> for CoreError {
    fn from(e: borsh::io::Error) -> Self {
        CoreError::TxParsingError(e.to_string())
    }
}
