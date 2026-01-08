use ssz::DecodeError;
use strata_l1_envelope_fmt::errors::EnvelopeParseError;
use thiserror::Error;

/// Errors that can occur while parsing checkpoint transactions from SPS-50 envelopes.
#[derive(Debug, Error)]
pub enum CheckpointTxError {
    /// Transaction did not contain any inputs.
    #[error("checkpoint transaction missing inputs")]
    MissingInputs,

    /// The taproot leaf script was not present in the first input witness.
    #[error("checkpoint transaction missing taproot leaf script in first input witness")]
    MissingLeafScript,

    /// Failed to parse the envelope script structure.
    #[error("failed to parse checkpoint envelope script: {0}")]
    EnvelopeParse(#[from] EnvelopeParseError),

    /// Failed to deserialize SSZ checkpoint payload.
    #[error("failed to deserialize checkpoint payload: {0:?}")]
    SszDecode(#[from] DecodeError),
}

/// Result alias for checkpoint transaction helpers.
pub type CheckpointTxResult<T> = Result<T, CheckpointTxError>;
