//! Withdrawal message types for bridge gateway account communication.

use strata_acct_types::VarVec;
use strata_codec::{Codec, CodecError, Decoder, Encoder};
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
    pub fees: u32,
    /// Bitcoin Output Script Descriptor describing the withdrawal output.
    pub dest_desc: VarVec<u8>,
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

    /// Get the fees.
    pub fn fees(&self) -> u32 {
        self.fees
    }

    /// Get the destination descriptor as bytes.
    pub fn dest_desc(&self) -> &[u8] {
        self.dest_desc.as_ref()
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WithdrawalFeeBumpMsgData {
    /// Index of the withdrawal intent to bump.
    pub withdrawal_intent_idx: u32,
    /// Source subject requesting the fee bump.
    pub source_subject: SubjectId,
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

impl Codec for WithdrawalFeeBumpMsgData {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        self.withdrawal_intent_idx.encode(enc)?;
        self.source_subject.encode(enc)?;
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let withdrawal_intent_idx = u32::decode(dec)?;
        let source_subject = SubjectId::decode(dec)?;
        Ok(Self {
            withdrawal_intent_idx,
            source_subject,
        })
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

/// Message data for withdrawal rejection from the bridge gateway account.
///
/// This message type occurs when the bridge gateway account rejects
/// a withdrawal initiation or a withdrawal fee bump.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WithdrawalRejectionMsgData {
    /// Index of the withdrawal intent.
    pub withdrawal_intent_idx: u32,
    /// Type of rejection.
    pub rejection_type: WithdrawalRejectionType,
    /// Source subject that initiated the withdrawal.
    pub source_subject: SubjectId,
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

impl Codec for WithdrawalRejectionMsgData {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        self.withdrawal_intent_idx.encode(enc)?;
        self.rejection_type.to_u8().encode(enc)?;
        self.source_subject.encode(enc)?;
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let withdrawal_intent_idx = u32::decode(dec)?;
        let rejection_type_u8 = u8::decode(dec)?;
        let rejection_type = WithdrawalRejectionType::from_u8(rejection_type_u8)
            .ok_or(CodecError::OverflowContainer)?;
        let source_subject = SubjectId::decode(dec)?;
        Ok(Self {
            withdrawal_intent_idx,
            rejection_type,
            source_subject,
        })
    }
}
