//! Inbox accumulator types for snark accounts.

use strata_acct_types::{AccountId, MsgPayload};
use strata_codec::{Codec, CodecError, Decoder, Encoder};
use strata_da_framework::LinearAccumulator;
use strata_snark_acct_types::MessageEntry;

use super::MAX_MSG_PAYLOAD_BYTES;

/// DA-encoded snark inbox message entry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DaMessageEntry {
    /// Source account for the message.
    pub source: AccountId,

    /// Epoch in which the message was included.
    pub incl_epoch: u32,

    /// Message payload.
    pub payload: MsgPayload,
}

impl From<MessageEntry> for DaMessageEntry {
    fn from(value: MessageEntry) -> Self {
        Self {
            source: value.source(),
            incl_epoch: value.incl_epoch(),
            payload: value.payload().clone(),
        }
    }
}

impl Codec for DaMessageEntry {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        self.source.encode(enc)?;
        self.incl_epoch.encode(enc)?;
        self.payload.encode(enc)?;
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let source = AccountId::decode(dec)?;
        let incl_epoch = u32::decode(dec)?;
        let payload = MsgPayload::decode(dec)?;
        if payload.data().len() > MAX_MSG_PAYLOAD_BYTES {
            return Err(CodecError::OverflowContainer);
        }
        Ok(Self {
            source,
            incl_epoch,
            payload,
        })
    }
}

/// Buffer of DA-encoded inbox messages for insertion into the real accumulator.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct InboxBuffer {
    /// Inbox entries appended during the epoch.
    entries: Vec<DaMessageEntry>,
}

impl InboxBuffer {
    pub fn entries(&self) -> &[DaMessageEntry] {
        &self.entries
    }
}

impl LinearAccumulator for InboxBuffer {
    type InsertCnt = u16;
    type EntryData = DaMessageEntry;
    const MAX_INSERT: Self::InsertCnt = u16::MAX;

    fn insert(&mut self, entry: &Self::EntryData) {
        self.entries.push(entry.clone());
    }
}
