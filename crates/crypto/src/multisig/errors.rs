use thiserror::Error;

use crate::multisig::PubKey;

/// Errors related to multisig configuration updates and thresholds.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum MultisigConfigError {
    /// A new member to be added already exists in the multisig configuration.
    #[error("cannot add member {0:?}: already exists in multisig configuration")]
    MemberAlreadyExists(PubKey),

    /// An old member to be removed was not found in the multisig configuration.
    #[error("cannot remove member {0:?}: not found in multisig configuration")]
    MemberNotFound(PubKey),

    /// The provided threshold is invalid (must be strictly greater than half of the multisig
    /// size).
    #[error(
        "invalid threshold {threshold}: must be greater than half of the multisig size ({min_required})"
    )]
    InvalidThreshold {
        /// The threshold value provided in the transaction.
        threshold: u8,
        /// The minimum valid threshold for this multisig (computed as `size / 2 + 1`).
        min_required: usize,
    },
}

/// Errors related to validating a multisig vote (aggregation or signature check).
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum VoteValidationError {
    /// Failed to aggregate public keys for multisig vote.
    #[error("failed to aggregate public keys for multisig vote")]
    AggregationError,

    /// The aggregated vote signature is invalid.
    #[error("invalid vote signature")]
    InvalidVoteSignature,
}
