//! Log payload types for orchestration layer logs.
//!
//! The canonical log-payload types shared with checkpoint verification
//! ([`OLLogType`], [`SimpleWithdrawalIntentLogData`], the type-id namespace, and
//! [`OLLogDecodeError`]) live in [`strata_asm_proto_checkpoint_types`] and are re-exported from the
//! crate root. This module only holds the OL-block-local log types that are not consumed by
//! checkpoint verification, such as [`SnarkAccountUpdateLogData`].

use strata_asm_proto_checkpoint_types::OLLogType;
use strata_codec::{Codec, VarVec};
use strata_msg_fmt::TypeId;

use crate::{SAU_MAX_EXTRA_DATA_BYTES, SauTxUpdateData};

/// Maximum byte length for snark account update extra data (matches
/// `SAU_MAX_EXTRA_DATA_BYTES` from the SSZ spec).
const MAX_EXTRA_DATA_BYTES: u32 = SAU_MAX_EXTRA_DATA_BYTES as u32;

/// Bounded [`VarVec`] holding SAU extra data.
pub type ExtraDataBufVec = VarVec<u8, { MAX_EXTRA_DATA_BYTES }>;

/// msg-fmt type id for [`SnarkAccountUpdateLogData`].
///
/// Shares the OL log type-id namespace with
/// [`SIMPLE_WITHDRAWAL_INTENT_LOG_TYPE_ID`](strata_asm_proto_checkpoint_types::SIMPLE_WITHDRAWAL_INTENT_LOG_TYPE_ID).
/// This log is emitted within OL blocks but is not consumed by checkpoint verification.
pub const SNARK_ACCOUNT_UPDATE_LOG_TYPE_ID: TypeId = 0x02;

/// Payload for a snark account update log.
///
/// This log is emitted when a snark account is updated through a transaction.
/// It contains the new message index (sequence number) and any extra data
/// from the update operation.
#[derive(Debug, Clone, PartialEq, Eq, Codec)]
pub struct SnarkAccountUpdateLogData {
    /// The new message index (sequence number) after the update.
    pub new_msg_idx: u64,

    /// Extra data from the update operation.
    pub extra_data: ExtraDataBufVec,
}

impl SnarkAccountUpdateLogData {
    /// Create a new snark account update log data instance.
    pub fn new(new_msg_idx: u64, extra_data: Vec<u8>) -> Option<Self> {
        VarVec::from_vec(extra_data).map(|extra_data| Self {
            new_msg_idx,
            extra_data,
        })
    }

    /// Create a new snark update log data from [`SauTxUpdateData`]
    pub fn from_sau_data(sau_data: &SauTxUpdateData) -> Option<Self> {
        let new_msg_idx = sau_data.proof_state().new_next_msg_idx();
        let extra_data = sau_data.extra_data().to_vec();
        Self::new(new_msg_idx, extra_data)
    }

    /// Get the new message index.
    pub fn new_msg_idx(&self) -> u64 {
        self.new_msg_idx
    }

    /// Get the extra data as bytes.
    pub fn extra_data(&self) -> &[u8] {
        self.extra_data.as_ref()
    }
}

impl OLLogType for SnarkAccountUpdateLogData {
    const TY: TypeId = SNARK_ACCOUNT_UPDATE_LOG_TYPE_ID;
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use strata_codec::{decode_buf_exact, encode_to_vec};

    use super::*;

    fn snark_update_strategy() -> impl Strategy<Value = SnarkAccountUpdateLogData> {
        (
            any::<u64>(),
            prop::collection::vec(any::<u8>(), 0..=MAX_EXTRA_DATA_BYTES as usize),
        )
            .prop_map(|(new_msg_idx, extra_data)| {
                SnarkAccountUpdateLogData::new(new_msg_idx, extra_data)
                    .expect("extra data within bounds")
            })
    }

    #[test]
    fn test_snark_account_update_log_data_codec() {
        // Create test data
        let log_data = SnarkAccountUpdateLogData {
            new_msg_idx: 12345,
            extra_data: VarVec::from_vec(b"extra_test_data".to_vec()).unwrap(),
        };

        // Encode
        let encoded = encode_to_vec(&log_data).unwrap();

        // Decode
        let decoded: SnarkAccountUpdateLogData = decode_buf_exact(&encoded).unwrap();

        // Verify round-trip
        assert_eq!(decoded.new_msg_idx, log_data.new_msg_idx);
        assert_eq!(decoded.extra_data.as_ref(), log_data.extra_data.as_ref());
    }

    #[test]
    fn test_snark_account_update_empty_extra_data() {
        // Test with empty extra data
        let log_data = SnarkAccountUpdateLogData {
            new_msg_idx: 999,
            extra_data: VarVec::from_vec(vec![]).unwrap(),
        };

        let encoded = encode_to_vec(&log_data).unwrap();
        let decoded: SnarkAccountUpdateLogData = decode_buf_exact(&encoded).unwrap();

        assert_eq!(decoded.new_msg_idx, 999);
        assert!(decoded.extra_data.is_empty());
    }

    #[test]
    fn test_snark_account_update_max_values() {
        // Test with maximum values
        let log_data = SnarkAccountUpdateLogData {
            new_msg_idx: u64::MAX,
            extra_data: VarVec::from_vec(vec![255u8; 250]).unwrap(),
        };

        let encoded = encode_to_vec(&log_data).unwrap();
        let decoded: SnarkAccountUpdateLogData = decode_buf_exact(&encoded).unwrap();

        assert_eq!(decoded.new_msg_idx, u64::MAX);
        assert_eq!(decoded.extra_data.len(), 250);
        assert_eq!(decoded.extra_data.as_ref(), &vec![255u8; 250][..]);
    }

    #[test]
    fn test_snark_account_update_zero_msg_idx() {
        // Test with zero message index
        let log_data = SnarkAccountUpdateLogData {
            new_msg_idx: 0,
            extra_data: VarVec::from_vec(b"test".to_vec()).unwrap(),
        };

        let encoded = encode_to_vec(&log_data).unwrap();
        let decoded: SnarkAccountUpdateLogData = decode_buf_exact(&encoded).unwrap();

        assert_eq!(decoded.new_msg_idx, 0);
        assert_eq!(decoded.extra_data.as_ref(), b"test");
    }

    proptest! {
        #[test]
        fn test_snark_update_codec_round_trip(log_data in snark_update_strategy()) {
            let encoded = encode_to_vec(&log_data).expect("encode should succeed");
            let decoded: SnarkAccountUpdateLogData =
                decode_buf_exact(&encoded).expect("decode should succeed");
            prop_assert_eq!(decoded, log_data);
        }
    }
}
