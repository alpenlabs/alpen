//! OL log types.
//!
//! The log container ([`OLLog`]), the typed payloads, and their envelope codec are now defined
//! canonically in [`strata_ol_logs`] (shared with the ASM checkpoint subprotocol); this module
//! re-exports them. The OL-STF-specific bridge from [`SauTxUpdateData`] lives on
//! [`SauTxUpdateData::get_log_data`].
//!
//! [`SauTxUpdateData`]: crate::SauTxUpdateData
//! [`SauTxUpdateData::get_log_data`]: crate::SauTxUpdateData::get_log_data

pub use strata_ol_logs::{
    DestinationBufVec, ExtraDataBufVec, LogDecodeError, MAX_LOG_PAYLOAD_LEN, OLLog, OLLogRef,
    OLLogType, SIMPLE_WITHDRAWAL_INTENT_LOG_TYPE_ID, SNARK_ACCOUNT_UPDATE_LOG_TYPE_ID,
    SimpleWithdrawalIntentLogData, SnarkAccountUpdateLogData, decode_typed_logs,
};
