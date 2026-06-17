//! OL log types.
//!
//! The log container ([`OLLog`]), the typed payloads, and their envelope codec are now defined
//! canonically in [`strata_ol_logs`] (shared with the ASM checkpoint subprotocol); this module
//! re-exports them and adds the OL-STF-specific bridge from [`SauTxUpdateData`].

pub use strata_ol_logs::{
    DestinationBufVec, ExtraDataBufVec, LogDecodeError, MAX_LOG_PAYLOAD_LEN, OLLog, OLLogRef,
    OLLogType, SIMPLE_WITHDRAWAL_INTENT_LOG_TYPE_ID, SNARK_ACCOUNT_UPDATE_LOG_TYPE_ID,
    SimpleWithdrawalIntentLogData, SnarkAccountUpdateLogData, decode_typed_logs,
};

use crate::SauTxUpdateData;

/// Builds a [`SnarkAccountUpdateLogData`] from a snark account update.
///
/// Returns `None` if the update's extra data exceeds the [`ExtraDataBufVec`] bound. The bound
/// matches the SSZ `SAU_MAX_EXTRA_DATA_BYTES` cap, so a well-formed update always fits.
pub fn snark_update_log_from_sau_data(
    sau_data: &SauTxUpdateData,
) -> Option<SnarkAccountUpdateLogData> {
    SnarkAccountUpdateLogData::new(
        sau_data.proof_state().new_next_msg_idx(),
        sau_data.extra_data().to_vec(),
    )
}
