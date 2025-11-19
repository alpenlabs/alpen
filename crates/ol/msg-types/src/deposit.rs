//! Deposit message types for bridge gateway account communication.

use strata_codec::{Codec, CodecError, Decoder, Encoder};
use strata_identifiers::SubjectId;

/// Message type ID for deposits.
pub const DEPOSIT_MSG_TYPE_ID: u16 = 0x02;

/// Message data for a deposit from the bridge gateway account.
///
/// This message type is sent by the bridge gateway account and represents
/// a simple deposit from L1 without a data payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DepositMsgData {
    /// The destination subject within the execution domain.
    pub dest_subject: SubjectId,
}

impl DepositMsgData {
    /// Create a new deposit message data instance.
    pub fn new(dest_subject: SubjectId) -> Self {
        Self { dest_subject }
    }

    /// Get the destination subject.
    pub fn dest_subject(&self) -> &SubjectId {
        &self.dest_subject
    }
}

impl Codec for DepositMsgData {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        self.dest_subject.encode(enc)?;
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let dest_subject = SubjectId::decode(dec)?;
        Ok(Self { dest_subject })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use strata_codec::{decode_buf_exact, encode_to_vec};

    #[test]
    fn test_deposit_msg_data_codec() {
        // Create test data
        let msg_data = DepositMsgData {
            dest_subject: SubjectId::from([42u8; 32]),
        };

        // Encode
        let encoded = encode_to_vec(&msg_data).unwrap();

        // Decode
        let decoded: DepositMsgData = decode_buf_exact(&encoded).unwrap();

        // Verify round-trip
        assert_eq!(decoded.dest_subject, msg_data.dest_subject);
    }

    #[test]
    fn test_deposit_msg_data_zero_subject() {
        // Test with zero subject
        let msg_data = DepositMsgData {
            dest_subject: SubjectId::from([0u8; 32]),
        };

        let encoded = encode_to_vec(&msg_data).unwrap();
        let decoded: DepositMsgData = decode_buf_exact(&encoded).unwrap();

        assert_eq!(decoded.dest_subject, SubjectId::from([0u8; 32]));
    }

    #[test]
    fn test_deposit_msg_data_max_subject() {
        // Test with max values
        let msg_data = DepositMsgData {
            dest_subject: SubjectId::from([255u8; 32]),
        };

        let encoded = encode_to_vec(&msg_data).unwrap();
        let decoded: DepositMsgData = decode_buf_exact(&encoded).unwrap();

        assert_eq!(decoded.dest_subject, SubjectId::from([255u8; 32]));
    }
}
