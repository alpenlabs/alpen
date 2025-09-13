use thiserror::Error;

use crate::multisig::PubKey;

/// Errors related to multisig configuration updates and thresholds.
#[derive(Clone, Debug, Eq, PartialEq, Error)]
pub enum MultisigConfigError {
    /// A new member to be added already exists in the multisig configuration.
    #[error("cannot add member {0:?}: already exists in multisig configuration")]
    MemberAlreadyExists(PubKey),

    /// An old member to be removed was not found in the multisig configuration.
    #[error("cannot remove member {0:?}: not found in multisig configuration")]
    MemberNotFound(PubKey),

    /// The provided threshold is invalid.
    #[error("invalid threshold {threshold}: must not exceed {total_keys}")]
    InvalidThreshold {
        /// The threshold value provided.
        threshold: u8,
        /// The total keys in the multisig.
        total_keys: usize,
    },

    /// The keys list is empty.
    #[error("keys cannot be empty")]
    EmptyKeys,
}

/// Errors related to validating a multisig vote (aggregation or signature check).
#[derive(Clone, Debug, Eq, PartialEq, Error)]
pub enum VoteValidationError {
    /// Failed to aggregate public keys for multisig vote.
    #[error("failed to aggregate public keys for multisig vote")]
    AggregationError,

    /// The aggregated vote signature is invalid.
    #[error("invalid vote signature")]
    InvalidVoteSignature,
}
