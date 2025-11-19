//! Log payload types for orchestration layer logs.

use strata_acct_types::VarVec;
use strata_codec::{Codec, CodecError, Decoder, Encoder};

/// Payload for a simple withdrawal intent log.
///
/// This log is emitted when a withdrawal intent is created through the bridge.
/// It contains the withdrawal amount and destination information.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimpleWithdrawalIntentLogData {
    /// Amount to withdraw (in satoshis).
    pub amt: u64,
    /// Destination address or script (BOSD - Bitcoin Output Script Descriptor).
    pub dest: VarVec<u8>,
}

impl SimpleWithdrawalIntentLogData {
    /// Create a new simple withdrawal intent log data instance.
    pub fn new(amt: u64, dest: Vec<u8>) -> Option<Self> {
        let dest = VarVec::from_vec(dest)?;
        Some(Self { amt, dest })
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

// Codec implementation for SimpleWithdrawalIntentLogData
impl Codec for SimpleWithdrawalIntentLogData {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        self.amt.encode(enc)?;
        self.dest.encode(enc)?;
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let amt = u64::decode(dec)?;
        let dest = VarVec::<u8>::decode(dec)?;
        Ok(Self { amt, dest })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use strata_codec::{decode_buf_exact, encode_to_vec};

    #[test]
    fn test_simple_withdrawal_intent_log_data_codec() {
        // Create test data
        let log_data = SimpleWithdrawalIntentLogData {
            amt: 100_000_000, // 1 BTC
            dest: VarVec::from_vec(b"bc1qtest123456789".to_vec()).unwrap(),
        };

        // Encode
        let encoded = encode_to_vec(&log_data).unwrap();

        // Decode
        let decoded: SimpleWithdrawalIntentLogData = decode_buf_exact(&encoded).unwrap();

        // Verify round-trip
        assert_eq!(decoded.amt, log_data.amt);
        assert_eq!(decoded.dest.as_ref(), log_data.dest.as_ref());
    }

    #[test]
    fn test_simple_withdrawal_intent_empty_dest() {
        // Test with empty destination (probably invalid, but codec should handle it)
        let log_data = SimpleWithdrawalIntentLogData {
            amt: 50_000,
            dest: VarVec::from_vec(vec![]).unwrap(),
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
        };

        let encoded = encode_to_vec(&log_data).unwrap();
        let decoded: SimpleWithdrawalIntentLogData = decode_buf_exact(&encoded).unwrap();

        assert_eq!(decoded.amt, 0);
        assert_eq!(decoded.dest.as_ref(), b"addr1test");
    }
}
