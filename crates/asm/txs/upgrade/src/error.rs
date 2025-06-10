use strata_primitives::buf::Buf32;
use thiserror::Error;

/// Top-level error type for the upgrade subprotocol, composed of smaller error categories.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum UpgradeTxParseError {
    /// Failed to deserialize the transaction payload for the given transaction type.
    #[error("failed to deserialize transaction for tx_type = {0}")]
    MalformedTransaction(u8),

    /// Failed to deserialize the transaction payload for the given transaction type.
    #[error("tx type is not defined")]
    UnknownTxType,
}

/// Errors related to multisig configuration updates and thresholds.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum MultisigConfigError {
    /// A new member to be added already exists in the multisig configuration.
    #[error("cannot add member {0:?}: already exists in multisig configuration")]
    MemberAlreadyExists(Buf32),

    /// An old member to be removed was not found in the multisig configuration.
    #[error("cannot remove member {0:?}: not found in multisig configuration")]
    MemberNotFound(Buf32),

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
