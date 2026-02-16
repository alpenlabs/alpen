use strata_asm_txs_admin::actions::UpdateId;
use strata_crypto::threshold_signature::ThresholdSignatureError;
use strata_primitives::roles::Role;
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
        "invalid seqno for {role:?}: payload seqno {payload_seqno} must be greater than \
         last seqno {last_seqno}"
    )]
    InvalidSeqno {
        role: Role,
        payload_seqno: u64,
        last_seqno: u64,
    },

    /// The gap between payload seqno and last seqno exceeds the configured maximum.
    #[error(
        "seqno gap too large for {role:?}: payload seqno {payload_seqno} exceeds \
         last seqno {last_seqno} by more than max gap {max_gap}"
    )]
    SeqnoGapTooLarge {
        role: Role,
        payload_seqno: u64,
        last_seqno: u64,
        max_gap: u8,
    },

    /// Indicates a threshold signature error (configuration or signature validation).
    #[error(transparent)]
    ThresholdSignature(#[from] ThresholdSignatureError),
}
