//! Withdrawal message types for bridge gateway account communication.

use strata_codec::{Codec, CodecError, Decoder, Encoder, VarVec};
use strata_identifiers::SubjectId;

/// Message type ID for withdrawal initiation.
pub const WITHDRAWAL_MSG_TYPE_ID: u16 = 0x03;

/// Message type ID for withdrawal fee bump.
pub const WITHDRAWAL_FEE_BUMP_MSG_TYPE_ID: u16 = 0x04;

/// Message type ID for withdrawal rejection.
pub const WITHDRAWAL_REJECTION_MSG_TYPE_ID: u16 = 0x05;

/// Maximum length for withdrawal destination descriptor.
pub const MAX_WITHDRAWAL_DESC_LEN: usize = 255;

/// Sentinel value indicating no selected operator for withdrawal assignment.
pub const NO_SELECTED_OPERATOR: u32 = u32::MAX;

// TODO: allow users to specify operator fee
pub const DEFAULT_OPERATOR_FEE: u32 = 0;

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
    // TODO idk why, but I can't make the MAX_WITHDRAWAL_DESC_LEN const generic work
    dest_desc: VarVec<u8>,

    /// User's selected operator index for withdrawal assignment.
    selected_operator: u32,
}

impl WithdrawalMsgData {
    /// Creates a new withdrawal message data instance.
    pub fn new(fees: u32, dest_desc: Vec<u8>, selected_operator: u32) -> Option<Self> {
        // Ensure the destination descriptor isn't too long.
        if dest_desc.len() > MAX_WITHDRAWAL_DESC_LEN {
            return None;
        }

        let dest_desc = VarVec::from_vec(dest_desc)?;
        Some(Self {
            fees,
            dest_desc,
            selected_operator,
        })
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

    /// Gets the user's selected operator index.
    pub fn selected_operator(&self) -> u32 {
        self.selected_operator
    }
}

impl Codec for WithdrawalMsgData {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        self.fees.encode(enc)?;
        self.dest_desc.encode(enc)?;
        self.selected_operator.encode(enc)?;
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let fees = u32::decode(dec)?;
        let dest_desc = VarVec::<u8>::decode(dec)?;

        // Validate the length constraint.
        //
        // TODO remove this when we fix the const generic type
        if dest_desc.len() > MAX_WITHDRAWAL_DESC_LEN {
            return Err(CodecError::OverflowContainer);
        }

        let selected_operator = u32::decode(dec)?;

        Ok(Self {
            fees,
            dest_desc,
            selected_operator,
        })
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
        Self::from_u8(dec.read_arr::<1>()?[0])
            .ok_or(CodecError::InvalidVariant("WithdrawalRejectionType"))
    }

    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        enc.write_buf(&[self.to_u8()])
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
    use proptest::prelude::*;
    use strata_codec::{decode_buf_exact, encode_to_vec};

    use super::*;

    proptest! {
        #[test]
        fn test_withdrawal_msg_data_codec(
            fees in 0u32..=u32::MAX,
            // Use a reasonable size limit for the descriptor (up to 255 bytes for safety)
            dest_desc_bytes in prop::collection::vec(any::<u8>(), 0..=255),
            selected_operator in any::<u32>(),
        ) {
            // Create test data with random values
            let dest_desc = VarVec::from_vec(dest_desc_bytes.clone())
                .expect("VarVec creation should succeed for valid byte vectors");

            let msg_data = WithdrawalMsgData {
                fees,
                dest_desc,
                selected_operator,
            };

            // Encode
            let encoded = encode_to_vec(&msg_data).expect("Encoding should succeed");

            // Decode
            let decoded: WithdrawalMsgData = decode_buf_exact(&encoded)
                .expect("Decoding should succeed");

            // Verify round-trip
            prop_assert_eq!(decoded.fees, msg_data.fees);
            prop_assert_eq!(decoded.dest_desc.as_ref(), msg_data.dest_desc.as_ref());
            prop_assert_eq!(decoded.selected_operator, msg_data.selected_operator);
        }
    }

    #[test]
    fn test_withdrawal_rejection_type_conversion() {
        // This just test all values exhaustively.
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
