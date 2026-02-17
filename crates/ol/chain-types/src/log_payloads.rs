//! Log payload types for orchestration layer logs.

use strata_codec::{Codec, VarVec};

/// Payload for a simple withdrawal intent log.
///
/// Emitted by the OL STF when a withdrawal message is processed at the bridge
/// gateway account.
#[derive(Debug, Clone, PartialEq, Eq, Codec)]
pub struct SimpleWithdrawalIntentLogData {
    /// Amount being withdrawn (sats).
    pub amt: u64,

    /// Destination BOSD.
    pub dest: VarVec<u8>,

    /// User's selected operator index for withdrawal assignment.
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
    pub extra_data: VarVec<u8>,
}

impl SnarkAccountUpdateLogData {
    /// Create a new snark account update log data instance.
    pub fn new(new_msg_idx: u64, extra_data: Vec<u8>) -> Option<Self> {
        let extra_data = VarVec::from_vec(extra_data)?;
        Some(Self {
            new_msg_idx,
            extra_data,
        })
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

#[cfg(test)]
mod tests {
    use strata_codec::{decode_buf_exact, encode_to_vec};

    use super::*;

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
}
