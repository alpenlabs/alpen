//! Log payload types for orchestration layer logs.

use std::iter;

use strata_codec::{Codec, CodecError, VarVec, decode_buf_exact};
use strata_msg_fmt::{Msg, MsgRef, TypeId, try_encode_into_buf};

use crate::{SAU_MAX_EXTRA_DATA_BYTES, SauTxUpdateData};

/// Maximum byte length for a withdrawal destination BOSD descriptor.
const MAX_DEST_BYTES: u32 = 255;

/// Maximum byte length for snark account update extra data (matches
/// `SAU_MAX_EXTRA_DATA_BYTES` from the SSZ spec).
const MAX_EXTRA_DATA_BYTES: u32 = SAU_MAX_EXTRA_DATA_BYTES as u32;

/// Bounded [`VarVec`] holding SAU extra data.
pub type ExtraDataBufVec = VarVec<u8, { MAX_EXTRA_DATA_BYTES }>;

/// Bounded [`VarVec`] holding withdrawal intent destination BOSD.
pub type DestinationBufVec = VarVec<u8, { MAX_DEST_BYTES }>;

/// msg-fmt type id for [`SimpleWithdrawalIntentLogData`].
pub const SIMPLE_WITHDRAWAL_INTENT_LOG_TYPE_ID: TypeId = 0x01;

/// msg-fmt type id for [`SnarkAccountUpdateLogData`].
pub const SNARK_ACCOUNT_UPDATE_LOG_TYPE_ID: TypeId = 0x02;

/// Error decoding a typed log payload from a msg-fmt message.
#[derive(Debug, thiserror::Error)]
pub enum LogDecodeError {
    /// The message's type id did not match the expected log type.
    #[error("log type mismatch (expected {0}, got {1})")]
    TypeMismatch(TypeId, TypeId),

    /// The message body failed to decode.
    #[error("failed to decode log body: {0}")]
    Decode(#[from] CodecError),
}

/// A typed orchestration-layer log payload.
///
/// Each impl carries a [`TypeId`] that tags it within an [`OLLog`](crate::OLLog) payload, encoded
/// using the `strata_msg_fmt` envelope (type prefix + SSZ body). This lets consumers parse a log
/// payload once and dispatch on its type id rather than guessing the concrete type from the raw
/// bytes.
pub trait OLLogType: Codec {
    /// The msg-fmt type id identifying this log payload.
    const LOG_TYPE_ID: TypeId;

    /// Encodes this payload into a msg-fmt envelope (type prefix followed by the SSZ body).
    fn encode_log(&self) -> Result<Vec<u8>, CodecError> {
        let mut buf = Vec::new();
        // Write the msg-fmt type prefix, then encode the body directly into the same buffer to
        // avoid a separate allocation and copy of the body.
        try_encode_into_buf(Self::LOG_TYPE_ID, iter::empty(), &mut buf)
            .expect("ol log: type id must be within msg-fmt bounds");
        self.encode(&mut buf)?;
        Ok(buf)
    }

    /// Attempts to decode this payload from a parsed msg-fmt message.
    ///
    /// Returns [`LogDecodeError::TypeMismatch`] if the message's type id does not match
    /// [`Self::LOG_TYPE_ID`], or [`LogDecodeError::Decode`] if the type id matches but the body
    /// fails to decode.
    fn try_decode_log(msg: &MsgRef<'_>) -> Result<Self, LogDecodeError> {
        if msg.ty() != Self::LOG_TYPE_ID {
            return Err(LogDecodeError::TypeMismatch(Self::LOG_TYPE_ID, msg.ty()));
        }
        Ok(decode_buf_exact(msg.body())?)
    }
}

/// Payload for a simple withdrawal intent log.
///
/// Emitted by the OL STF when a withdrawal message is processed at the bridge
/// gateway account.
#[derive(Debug, Clone, PartialEq, Eq, Codec)]
pub struct SimpleWithdrawalIntentLogData {
    /// Amount being withdrawn (sats).
    pub amt: u64,

    /// Destination BOSD.
    pub dest: DestinationBufVec,

    /// User's selected operator index for withdrawal assignment.
    // TODO(STR-1861): encode as varint to reduce DA cost in checkpoint payloads.
    pub selected_operator: u32,
}

impl SimpleWithdrawalIntentLogData {
    /// Create a new simple withdrawal intent log data instance.
    pub fn new(amt: u64, dest: Vec<u8>, selected_operator: u32) -> Option<Self> {
        let dest = VarVec::from_vec(dest)?;
        Some(Self {
            amt,
            dest,
            selected_operator,
        })
    }

    /// Get the withdrawal amount.
    pub fn amt(&self) -> u64 {
        self.amt
    }

    /// Get the destination as bytes.
    pub fn dest(&self) -> &[u8] {
        self.dest.as_ref()
    }
}

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

impl OLLogType for SimpleWithdrawalIntentLogData {
    const LOG_TYPE_ID: TypeId = SIMPLE_WITHDRAWAL_INTENT_LOG_TYPE_ID;
}

impl OLLogType for SnarkAccountUpdateLogData {
    const LOG_TYPE_ID: TypeId = SNARK_ACCOUNT_UPDATE_LOG_TYPE_ID;
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use strata_codec::{decode_buf_exact, encode_to_vec};

    use super::*;

    fn withdrawal_strategy() -> impl Strategy<Value = SimpleWithdrawalIntentLogData> {
        (
            any::<u64>(),
            prop::collection::vec(any::<u8>(), 0..=MAX_DEST_BYTES as usize),
            any::<u32>(),
        )
            .prop_map(|(amt, dest, selected_operator)| {
                SimpleWithdrawalIntentLogData::new(amt, dest, selected_operator)
                    .expect("dest within bounds")
            })
    }

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
    fn test_simple_withdrawal_intent_log_data_codec() {
        // Create test data
        let log_data = SimpleWithdrawalIntentLogData {
            amt: 100_000_000, // 1 BTC
            dest: VarVec::from_vec(b"bc1qtest123456789".to_vec()).unwrap(),
            selected_operator: 42,
        };

        // Encode
        let encoded = encode_to_vec(&log_data).unwrap();

        // Decode
        let decoded: SimpleWithdrawalIntentLogData = decode_buf_exact(&encoded).unwrap();

        // Verify round-trip
        assert_eq!(decoded.amt, log_data.amt);
        assert_eq!(decoded.dest.as_ref(), log_data.dest.as_ref());
        assert_eq!(decoded.selected_operator, log_data.selected_operator);
    }

    #[test]
    fn test_simple_withdrawal_intent_empty_dest() {
        // Test with empty destination (probably invalid, but codec should handle it)
        let log_data = SimpleWithdrawalIntentLogData {
            amt: 50_000,
            dest: VarVec::from_vec(vec![]).unwrap(),
            selected_operator: 0,
        };

        let encoded = encode_to_vec(&log_data).unwrap();
        let decoded: SimpleWithdrawalIntentLogData = decode_buf_exact(&encoded).unwrap();

        assert_eq!(decoded.amt, 50_000);
        assert!(decoded.dest.is_empty());
    }

    #[test]
    fn test_simple_withdrawal_intent_max_values() {
        // Test with maximum values
        let log_data = SimpleWithdrawalIntentLogData {
            amt: u64::MAX,
            dest: VarVec::from_vec(vec![255u8; 200]).unwrap(),
            selected_operator: u32::MAX,
        };

        let encoded = encode_to_vec(&log_data).unwrap();
        let decoded: SimpleWithdrawalIntentLogData = decode_buf_exact(&encoded).unwrap();

        assert_eq!(decoded.amt, u64::MAX);
        assert_eq!(decoded.dest.len(), 200);
        assert_eq!(decoded.dest.as_ref(), &vec![255u8; 200][..]);
    }

    #[test]
    fn test_simple_withdrawal_intent_zero_amount() {
        // Test with zero amount
        let log_data = SimpleWithdrawalIntentLogData {
            amt: 0,
            dest: VarVec::from_vec(b"addr1test".to_vec()).unwrap(),
            selected_operator: 5,
        };

        let encoded = encode_to_vec(&log_data).unwrap();
        let decoded: SimpleWithdrawalIntentLogData = decode_buf_exact(&encoded).unwrap();

        assert_eq!(decoded.amt, 0);
        assert_eq!(decoded.dest.as_ref(), b"addr1test");
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
    fn test_log_envelope_round_trip() {
        let snark = SnarkAccountUpdateLogData {
            new_msg_idx: 7,
            extra_data: VarVec::from_vec(b"abc".to_vec()).unwrap(),
        };
        let encoded = snark.encode_log().unwrap();

        // Envelope carries the type id prefix.
        let msg = MsgRef::try_from(encoded.as_slice()).unwrap();
        assert_eq!(msg.ty(), SNARK_ACCOUNT_UPDATE_LOG_TYPE_ID);

        let decoded = SnarkAccountUpdateLogData::try_decode_log(&msg).unwrap();
        assert_eq!(decoded, snark);
    }

    #[test]
    fn test_log_decode_type_mismatch() {
        let withdrawal = SimpleWithdrawalIntentLogData {
            amt: 10,
            dest: VarVec::from_vec(b"d".to_vec()).unwrap(),
            selected_operator: 1,
        };
        let encoded = withdrawal.encode_log().unwrap();
        let msg = MsgRef::try_from(encoded.as_slice()).unwrap();

        // Decoding as the wrong log type reports a type mismatch rather than a spurious decode.
        let err = SnarkAccountUpdateLogData::try_decode_log(&msg).unwrap_err();
        assert!(matches!(
            err,
            LogDecodeError::TypeMismatch(SNARK_ACCOUNT_UPDATE_LOG_TYPE_ID, ty)
                if ty == SIMPLE_WITHDRAWAL_INTENT_LOG_TYPE_ID
        ));

        // And decoding as the right type still works.
        SimpleWithdrawalIntentLogData::try_decode_log(&msg).unwrap();
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
        fn test_withdrawal_log_envelope_round_trip(log_data in withdrawal_strategy()) {
            let encoded = log_data.encode_log().expect("encode_log should succeed");

            let msg = MsgRef::try_from(encoded.as_slice()).expect("envelope should parse");
            prop_assert_eq!(msg.ty(), SIMPLE_WITHDRAWAL_INTENT_LOG_TYPE_ID);

            let decoded = SimpleWithdrawalIntentLogData::try_decode_log(&msg)
                .expect("try_decode_log should succeed");
            prop_assert_eq!(decoded, log_data);
        }

        #[test]
        fn test_snark_update_log_envelope_round_trip(log_data in snark_update_strategy()) {
            let encoded = log_data.encode_log().expect("encode_log should succeed");

            let msg = MsgRef::try_from(encoded.as_slice()).expect("envelope should parse");
            prop_assert_eq!(msg.ty(), SNARK_ACCOUNT_UPDATE_LOG_TYPE_ID);

            let decoded = SnarkAccountUpdateLogData::try_decode_log(&msg)
                .expect("try_decode_log should succeed");
            prop_assert_eq!(decoded, log_data);
        }

        /// Decoding a withdrawal envelope as a snark update (and vice versa) reports a type
        /// mismatch rather than a spurious decode.
        #[test]
        fn test_withdrawal_log_decode_type_mismatch(log_data in withdrawal_strategy()) {
            let encoded = log_data.encode_log().expect("encode_log should succeed");
            let msg = MsgRef::try_from(encoded.as_slice()).expect("envelope should parse");

            let err = SnarkAccountUpdateLogData::try_decode_log(&msg)
                .expect_err("decoding as the wrong type should fail");
            prop_assert!(matches!(
                err,
                LogDecodeError::TypeMismatch(SNARK_ACCOUNT_UPDATE_LOG_TYPE_ID, ty)
                    if ty == SIMPLE_WITHDRAWAL_INTENT_LOG_TYPE_ID
            ));
        }

        #[test]
        fn test_snark_update_log_decode_type_mismatch(log_data in snark_update_strategy()) {
            let encoded = log_data.encode_log().expect("encode_log should succeed");
            let msg = MsgRef::try_from(encoded.as_slice()).expect("envelope should parse");

            let err = SimpleWithdrawalIntentLogData::try_decode_log(&msg)
                .expect_err("decoding as the wrong type should fail");
            prop_assert!(matches!(
                err,
                LogDecodeError::TypeMismatch(SIMPLE_WITHDRAWAL_INTENT_LOG_TYPE_ID, ty)
                    if ty == SNARK_ACCOUNT_UPDATE_LOG_TYPE_ID
            ));
        }
    }
}
