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
