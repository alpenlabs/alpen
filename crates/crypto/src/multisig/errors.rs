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

    /// The provided threshold is invalid.
    #[error("invalid threshold {threshold}: must be between {min_required} and {max_allowed}")]
    InvalidThreshold {
        /// The threshold value provided.
        threshold: u8,
        /// The minimum valid threshold.
        min_required: usize,
        /// The maximum valid threshold.
        max_allowed: usize,
    },

    /// The keys list is empty.
    #[error("keys cannot be empty")]
    EmptyKeys,
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
