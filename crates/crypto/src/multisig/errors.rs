use thiserror::Error;

/// Single error type for all multisig operations across all cryptographic schemes.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum MultisigError {
    /// Insufficient keys selected for aggregation.
    #[error("insufficient keys selected: provided {provided}, required at least {required}")]
    InsufficientKeys {
        /// Number of keys provided.
        provided: usize,
        /// Number of keys required.
        required: usize,
    },

    /// Invalid public key at a specific index.
    #[error("invalid public key at index {index}: {reason}")]
    InvalidPubKey {
        /// The index of the invalid key.
        index: usize,
        /// The reason why the key is invalid.
        reason: String,
    },

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

    /// The aggregated signature is invalid.
    #[error("invalid signature")]
    InvalidSignature,

    /// Key aggregation context creation failed.
    #[error("key aggregation context creation failed: {reason}")]
    AggregationContextFailed {
        /// The reason why context creation failed.
        reason: String,
    },

    /// A new member to be added already exists in the multisig configuration.
    #[error("cannot add member: already exists in multisig configuration")]
    MemberAlreadyExists,
}
