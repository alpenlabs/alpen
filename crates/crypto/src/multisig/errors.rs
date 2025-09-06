use musig2::errors::KeyAggError;
use secp256k1;
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

    #[error("invalid key")]
    InvalidPubKey(PubKey),

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

    /// Insufficient keys selected for aggregation.
    #[error("insufficient keys selected: provided {provided}, required at least {required}")]
    InsufficientKeys {
        /// Number of keys provided.
        provided: usize,
        /// Number of keys required.
        required: usize,
    },

    /// Key aggregation failed.
    #[error("key aggregation failed: {0}")]
    KeyAggregationFailed(#[from] KeyAggregationError),
}

/// Errors related to key aggregation.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum KeyAggregationError {
    /// Invalid x-only public key at a specific index.
    #[error("invalid x-only public key at index {index}: {source}")]
    InvalidXOnlyKey {
        /// The index of the invalid key.
        index: usize,
        /// The underlying secp256k1 error.
        #[source]
        source: secp256k1::Error,
    },

    /// Failed to create key aggregation context.
    #[error("failed to create key aggregation context: {0}")]
    ContextCreationFailed(#[from] KeyAggError),
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
