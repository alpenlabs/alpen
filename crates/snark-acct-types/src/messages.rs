//! Snark account types.

use strata_acct_types::{AcctId, MsgPayload};

// TODO use actual MMR proofs
type MmrProof = Vec<u8>;

/// Message entry in an account inbox.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MessageEntry {
    /// The source account ID of the message.
    source: AcctId,

    /// The epoch that the message was included.
    incl_epoch: u32,

    /// The message payload.
    payload: MsgPayload,
}

impl MessageEntry {
    pub fn new(source: AcctId, incl_epoch: u32, payload: MsgPayload) -> Self {
        Self {
            source,
            incl_epoch,
            payload,
        }
    }

    pub fn source(&self) -> AcctId {
        self.source
    }

    pub fn incl_epoch(&self) -> u32 {
        self.incl_epoch
    }

    pub fn payload(&self) -> &MsgPayload {
        &self.payload
    }

    /// Gets the data payload buf.
    pub fn payload_buf(&self) -> &[u8] {
        self.payload().data()
    }

    /// Gets the payload value.
    pub fn payload_value(&self) -> u64 {
        self.payload().value()
    }
}

/// Proof for a message in an inbox MMR.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MessageEntryProof {
    entry: MessageEntry,
    proof: MmrProof,
}

impl MessageEntryProof {
    pub fn new(entry: MessageEntry, proof: MmrProof) -> Self {
        Self { entry, proof }
    }

    pub fn entry(&self) -> &MessageEntry {
        &self.entry
    }

    pub fn proof(&self) -> &[u8] {
        &self.proof
    }
}
