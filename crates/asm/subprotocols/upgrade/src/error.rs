use thiserror::Error;

use crate::{actions::ActionId, crypto::PubKey};

/// Top-level error type for the upgrade subprotocol, composed of smaller error categories.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum UpgradeError {
    /// Indicates a deserialization/parse failure.
    #[error(transparent)]
    Deserialize(#[from] DeserializeError),

    /// The specified role is not recognized.
    #[error("the specified role is not recognized")]
    UnknownRole,

    /// The specified action ID does not correspond to any pending upgrade.
    #[error("no pending upgrade found for action_id = {0:?}")]
    UnknownAction(ActionId),

    /// Indicates a validation failure on multisig configuration or threshold.
    #[error(transparent)]
    MultisigConfig(#[from] MultisigConfigError),

    /// Indicates a failure when validating a vote.
    #[error(transparent)]
    Vote(#[from] VoteValidationError),
}

/// Errors related to multisig configuration updates and thresholds.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum MultisigConfigError {
    /// A new member to be added already exists in the multisig configuration.
    #[error("cannot add member {0:?}: already exists in multisig configuration")]
    MemberAlreadyExists(PubKey),

    /// An old member to be removed was not found in the multisig configuration.
    #[error("cannot remove member {0:?}: not found in multisig configuration")]
    MemberNotFound(PubKey),

    /// The provided threshold is invalid (must be strictly greater than half of the multisig size).
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

/// Errors related to failing to parse incoming transactions.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum DeserializeError {
    /// Failed to deserialize the transaction payload for the given transaction type.
    #[error("failed to deserialize transaction for tx_type = {0}")]
    MalformedTransaction(u8),
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
