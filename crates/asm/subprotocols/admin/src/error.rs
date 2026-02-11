use strata_asm_txs_admin::actions::UpdateId;
use strata_crypto::threshold_signature::ThresholdSignatureError;
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

    /// The payload's sequence number is not greater than the last executed sequence number.
    #[error(
        "invalid seqno: payload seqno {payload_seqno} must be greater than last seqno {last_seqno}"
    )]
    InvalidSeqno { payload_seqno: u64, last_seqno: u64 },

    /// Indicates a threshold signature error (configuration or signature validation).
    #[error(transparent)]
    ThresholdSignature(#[from] ThresholdSignatureError),
}
