//! Snark account types.

use strata_acct_types::AcctId;

// TODO use actual MMR proofs
type MmrProof = Vec<u8>;

/// Message entry in an account inbox.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MessageEntry {
    source: AcctId,
    incl_epoch: u32,
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

    pub fn payload_buf(&self) -> &[u8] {
        self.payload().data()
    }

    pub fn payload_value(&self) -> u64 {
        self.payload().value()
    }
}

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

/// The actual payload carried in a message.
///
/// This is both data and some native asset value.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct MsgPayload {
    value: u64,
    data: Vec<u8>,
}

impl MsgPayload {
    pub fn new(value: u64, data: Vec<u8>) -> Self {
        Self { value, data }
    }

    pub fn value(&self) -> u64 {
        self.value
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }
}
