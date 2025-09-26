use strata_asm_txs_admin::actions::UpdateId;
use strata_crypto::multisig::MultisigError;
use thiserror::Error;

/// Top-level error type for the administration subprotocol, composed of smaller error categories.
#[derive(Clone, Debug, Eq, PartialEq, Error)]
pub enum AdministrationError {
    /// The specified role is not recognized.
    #[error("the specified role is not recognized")]
    UnknownRole,

    /// The specified action ID does not correspond to any pending update.
    #[error("no pending update found for action_id = {0:?}")]
    UnknownAction(UpdateId),

    /// Indicates a multisig error (configuration, aggregation, or signature validation).
    #[error(transparent)]
    Multisig(#[from] MultisigError),
}
