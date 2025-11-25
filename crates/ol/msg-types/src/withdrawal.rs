//! Withdrawal message types for bridge gateway account communication.

use strata_acct_types::VarVec;
use strata_codec::{Codec, CodecError, Decoder, Encoder};
use strata_codec_derive::Codec;
use strata_identifiers::SubjectId;

/// Message type ID for withdrawal initiation.
pub const WITHDRAWAL_MSG_TYPE_ID: u16 = 0x03;

/// Message type ID for withdrawal fee bump.
pub const WITHDRAWAL_FEE_BUMP_MSG_TYPE_ID: u16 = 0x04;

/// Message type ID for withdrawal rejection.
pub const WITHDRAWAL_REJECTION_MSG_TYPE_ID: u16 = 0x05;

/// Maximum length for withdrawal destination descriptor.
pub const MAX_WITHDRAWAL_DESC_LEN: usize = 255;

/// Message data for withdrawal initiation to the bridge gateway account.
///
/// This message type is sent by accounts that want to trigger a withdrawal.
/// The value sent with the message should be equal to the predetermined
/// static withdrawal size.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WithdrawalMsgData {
    /// Fees in satoshis to be paid to the operator.
    ///
    /// Currently, this is just ignored.
    fees: u32,

    /// Bitcoin Output Script Descriptor describing the withdrawal output.
    dest_desc: VarVec<u8>,
}

impl WithdrawalMsgData {
    /// Create a new withdrawal message data instance.
    pub fn new(fees: u32, dest_desc: Vec<u8>) -> Option<Self> {
        // Ensure the destination descriptor isn't too long
        if dest_desc.len() > MAX_WITHDRAWAL_DESC_LEN {
            return None;
        }

        let dest_desc = VarVec::from_vec(dest_desc)?;
        Some(Self { fees, dest_desc })
    }

    /// Get the fees paid to the operator, in sats.
    pub fn fees(&self) -> u32 {
        self.fees
    }

    /// Get the destination descriptor as bytes.
    pub fn dest_desc(&self) -> &[u8] {
        self.dest_desc.as_ref()
    }

    /// Takes out the inner destination descriptor as a `VarVec`.
    pub fn into_dest_desc(self) -> VarVec<u8> {
        self.dest_desc
    }
}

impl Codec for WithdrawalMsgData {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        self.fees.encode(enc)?;
        self.dest_desc.encode(enc)?;
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let fees = u32::decode(dec)?;
        let dest_desc = VarVec::<u8>::decode(dec)?;

        // Validate the length constraint
        if dest_desc.len() > MAX_WITHDRAWAL_DESC_LEN {
            return Err(CodecError::OverflowContainer);
        }

        Ok(Self { fees, dest_desc })
    }
}

/// Message data for withdrawal fee bump.
///
/// This message type is sent by accounts that want to bump the fee for
/// a pending withdrawal in the withdrawal intents queue.
///
/// This is currently unused and unsupported.
#[derive(Debug, Clone, PartialEq, Eq, Codec)]
pub struct WithdrawalFeeBumpMsgData {
    /// Index of the withdrawal intent to bump.
    withdrawal_intent_idx: u32,

    /// Source subject requesting the fee bump.
    source_subject: SubjectId,
}

impl WithdrawalFeeBumpMsgData {
    /// Create a new withdrawal fee bump message data instance.
    pub fn new(withdrawal_intent_idx: u32, source_subject: SubjectId) -> Self {
        Self {
            withdrawal_intent_idx,
            source_subject,
        }
    }

    /// Get the withdrawal intent index.
    pub fn withdrawal_intent_idx(&self) -> u32 {
        self.withdrawal_intent_idx
    }

    /// Get the source subject.
    pub fn source_subject(&self) -> &SubjectId {
        &self.source_subject
    }
}

/// Rejection type for withdrawal rejection messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WithdrawalRejectionType {
    /// The withdrawal was rejected entirely.
    RejectedEntirely = 0,

    /// This is a withdrawal fee bump rejection.
    FeeBumpRejection = 1,
}

impl WithdrawalRejectionType {
    /// Convert from u8 representation.
    pub fn from_u8(val: u8) -> Option<Self> {
        match val {
            0 => Some(Self::RejectedEntirely),
            1 => Some(Self::FeeBumpRejection),
            _ => None,
        }
    }

    /// Convert to u8 representation.
    pub fn to_u8(self) -> u8 {
        self as u8
    }
}

impl Codec for WithdrawalRejectionType {
    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        match dec.read_arr::<1>()?[0] {
            0 => Ok(Self::RejectedEntirely),
            1 => Ok(Self::FeeBumpRejection),
            _ => Err(CodecError::InvalidVariant("WithdrawalRejectionType")),
        }
    }

    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        let b = match self {
            Self::RejectedEntirely => 0,
            Self::FeeBumpRejection => 1,
        };
        enc.write_buf(&[b])
    }
}

/// Message data for withdrawal rejection from the bridge gateway account.
///
/// This message type occurs when the bridge gateway account rejects
/// a withdrawal initiation or a withdrawal fee bump.
///
/// This is currently unused.
#[derive(Debug, Clone, PartialEq, Eq, Codec)]
pub struct WithdrawalRejectionMsgData {
    /// Index of the withdrawal intent.
    withdrawal_intent_idx: u32,

    /// Type of rejection.
    rejection_type: WithdrawalRejectionType,

    /// Source subject that initiated the withdrawal.
    source_subject: SubjectId,
}

impl WithdrawalRejectionMsgData {
    /// Create a new withdrawal rejection message data instance.
    pub fn new(
        withdrawal_intent_idx: u32,
        rejection_type: WithdrawalRejectionType,
        source_subject: SubjectId,
    ) -> Self {
        Self {
            withdrawal_intent_idx,
            rejection_type,
            source_subject,
        }
    }

    /// Get the withdrawal intent index.
    pub fn withdrawal_intent_idx(&self) -> u32 {
        self.withdrawal_intent_idx
    }

    /// Get the rejection type.
    pub fn rejection_type(&self) -> WithdrawalRejectionType {
        self.rejection_type
    }

    /// Get the source subject.
    pub fn source_subject(&self) -> &SubjectId {
        &self.source_subject
    }
}

#[cfg(test)]
mod tests {
    use strata_codec::{decode_buf_exact, encode_to_vec};

    use super::*;

    #[test]
    fn test_withdrawal_msg_data_codec() {
        // Create test data
        let msg_data = WithdrawalMsgData {
            fees: 10_000,
            dest_desc: VarVec::from_vec(b"bc1qtest123".to_vec()).unwrap(),
        };

        // Encode
        let encoded = encode_to_vec(&msg_data).unwrap();

        // Decode
        let decoded: WithdrawalMsgData = decode_buf_exact(&encoded).unwrap();

        // Verify round-trip
        assert_eq!(decoded.fees, msg_data.fees);
        assert_eq!(decoded.dest_desc.as_ref(), msg_data.dest_desc.as_ref());
    }

    #[test]
    fn test_withdrawal_msg_data_empty_desc() {
        // Test with empty descriptor
        let msg_data = WithdrawalMsgData {
            fees: 0,
            dest_desc: VarVec::from_vec(vec![]).unwrap(),
        };

        let encoded = encode_to_vec(&msg_data).unwrap();
        let decoded: WithdrawalMsgData = decode_buf_exact(&encoded).unwrap();

        assert_eq!(decoded.fees, 0);
        assert!(decoded.dest_desc.is_empty());
    }

    #[test]
    fn test_withdrawal_msg_data_max_fees() {
        // Test with max fees
        let msg_data = WithdrawalMsgData {
            fees: u32::MAX,
            dest_desc: VarVec::from_vec(vec![255u8; 100]).unwrap(),
        };

        let encoded = encode_to_vec(&msg_data).unwrap();
        let decoded: WithdrawalMsgData = decode_buf_exact(&encoded).unwrap();

        assert_eq!(decoded.fees, u32::MAX);
        assert_eq!(decoded.dest_desc.len(), 100);
    }

    #[test]
    fn test_withdrawal_fee_bump_msg_data_codec() {
        // Create test data
        let msg_data = WithdrawalFeeBumpMsgData {
            withdrawal_intent_idx: 42,
            source_subject: SubjectId::from([1u8; 32]),
        };

        // Encode
        let encoded = encode_to_vec(&msg_data).unwrap();

        // Decode
        let decoded: WithdrawalFeeBumpMsgData = decode_buf_exact(&encoded).unwrap();

        // Verify round-trip
        assert_eq!(
            decoded.withdrawal_intent_idx,
            msg_data.withdrawal_intent_idx
        );
        assert_eq!(decoded.source_subject, msg_data.source_subject);
    }

    #[test]
    fn test_withdrawal_rejection_msg_data_codec() {
        // Test with RejectedEntirely
        let msg_data = WithdrawalRejectionMsgData {
            withdrawal_intent_idx: 100,
            rejection_type: WithdrawalRejectionType::RejectedEntirely,
            source_subject: SubjectId::from([2u8; 32]),
        };

        let encoded = encode_to_vec(&msg_data).unwrap();
        let decoded: WithdrawalRejectionMsgData = decode_buf_exact(&encoded).unwrap();

        assert_eq!(
            decoded.withdrawal_intent_idx,
            msg_data.withdrawal_intent_idx
        );
        assert_eq!(
            decoded.rejection_type,
            WithdrawalRejectionType::RejectedEntirely
        );
        assert_eq!(decoded.source_subject, msg_data.source_subject);
    }

    #[test]
    fn test_withdrawal_rejection_fee_bump_codec() {
        // Test with FeeBumpRejection
        let msg_data = WithdrawalRejectionMsgData {
            withdrawal_intent_idx: 0,
            rejection_type: WithdrawalRejectionType::FeeBumpRejection,
            source_subject: SubjectId::from([255u8; 32]),
        };

        let encoded = encode_to_vec(&msg_data).unwrap();
        let decoded: WithdrawalRejectionMsgData = decode_buf_exact(&encoded).unwrap();

        assert_eq!(decoded.withdrawal_intent_idx, 0);
        assert_eq!(
            decoded.rejection_type,
            WithdrawalRejectionType::FeeBumpRejection
        );
        assert_eq!(decoded.source_subject, SubjectId::from([255u8; 32]));
    }

    #[test]
    fn test_withdrawal_rejection_type_conversion() {
        // Test type conversions
        assert_eq!(
            WithdrawalRejectionType::from_u8(0),
            Some(WithdrawalRejectionType::RejectedEntirely)
        );
        assert_eq!(
            WithdrawalRejectionType::from_u8(1),
            Some(WithdrawalRejectionType::FeeBumpRejection)
        );
        assert_eq!(WithdrawalRejectionType::from_u8(2), None);

        assert_eq!(WithdrawalRejectionType::RejectedEntirely.to_u8(), 0);
        assert_eq!(WithdrawalRejectionType::FeeBumpRejection.to_u8(), 1);
    }
}
