use strata_asm_proto_upgrade_txs::actions::UpdateId;
use strata_crypto::multisig::errors::{MultisigConfigError, VoteValidationError};
use thiserror::Error;

/// Top-level error type for the upgrade subprotocol, composed of smaller error categories.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum UpgradeError {
    /// Indicates a deserialization/parse failure.
    #[error(transparent)]
    Deserialize(#[from] DeserializeError),

    /// The specified role is not recognized.
    #[error("the specified role is not recognized")]
    UnknownRole,

    /// The specified action ID does not correspond to any pending update.
    #[error("no pending update found for action_id = {0:?}")]
    UnknownAction(UpdateId),

    /// Indicates a validation failure on multisig configuration or threshold.
    #[error(transparent)]
    MultisigConfig(#[from] MultisigConfigError),

    /// Indicates a failure when validating a vote.
    #[error(transparent)]
    Vote(#[from] VoteValidationError),

    /// Indicates a failure when validating a vote.
    #[error(transparent)]
    Action(#[from] UpdateActionError),
}

/// Errors related to upgrade action.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum UpdateActionError {
    /// The update action cannot be queued.
    #[error("update action cannot be queued")]
    CannotQueue,

    /// The update action cannot be directly scheduled.
    #[error("update action cannot be directly scheduled")]
    CannotSchedule,
}

/// Errors related to failing to parse incoming transactions.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum DeserializeError {
    /// Failed to deserialize the transaction payload for the given transaction type.
    #[error("failed to deserialize transaction for tx_type = {0}")]
    MalformedTransaction(u8),
}
