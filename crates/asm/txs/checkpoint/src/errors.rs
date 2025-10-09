use borsh::io::Error as BorshIoError;
use strata_l1_envelope_fmt::errors::EnvelopeParseError;
use thiserror::Error;

/// Errors that can occur while parsing checkpoint transactions from SPS-50 envelopes.
#[derive(Debug, Error)]
pub enum CheckpointTxError {
    /// Encountered an SPS-50 tag with an unexpected transaction type.
    #[error("unsupported checkpoint tx type: expected {expected}, got {actual}")]
    UnexpectedTxType { expected: u8, actual: u8 },

    /// Transaction did not contain any inputs.
    #[error("checkpoint transaction missing inputs")]
    MissingInputs,

    /// The taproot leaf script was not present in the first input witness.
    #[error("checkpoint transaction missing taproot leaf script in first input witness")]
    MissingLeafScript,

    /// Failed to parse the envelope script structure.
    #[error("failed to parse checkpoint envelope script: {0}")]
    EnvelopeParse(#[from] EnvelopeParseError),

    /// Failed to deserialize data embedded inside the envelope.
    #[error("failed to deserialize checkpoint payload")]
    Deserialization(#[source] BorshIoError),
}

/// Result alias for checkpoint transaction helpers.
pub type CheckpointTxResult<T> = Result<T, CheckpointTxError>;
