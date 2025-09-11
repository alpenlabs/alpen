use strata_asm_proto_administration_txs::actions::UpdateId;
use strata_crypto::multisig::errors::{MultisigConfigError, VoteValidationError};
use thiserror::Error;

/// Top-level error type for the administration subprotocol, composed of smaller error categories.
#[derive(Clone, Debug, Eq, PartialEq, Error)]
pub(crate) enum AdministrationError {
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
}
