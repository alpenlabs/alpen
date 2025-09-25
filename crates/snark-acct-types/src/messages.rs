//! Snark account types.

use strata_acct_types::{AccountId, BitcoinAmount, MsgPayload, RawMerkleProof};

/// Message entry in an account inbox.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MessageEntry {
    /// The source account ID of the message.
    source: AccountId,

    /// The epoch that the message was included.
    incl_epoch: u32,

    /// The message payload.
    payload: MsgPayload,
}

impl MessageEntry {
    pub fn new(source: AccountId, incl_epoch: u32, payload: MsgPayload) -> Self {
        Self {
            source,
            incl_epoch,
            payload,
        }
    }

    pub fn source(&self) -> AccountId {
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
    pub fn payload_value(&self) -> BitcoinAmount {
        self.payload().value()
    }
}

/// Proof for a message in an inbox MMR.
///
/// This message entry doesn't imply a specific index, since this is implicit
/// from context.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MessageEntryProof {
    entry: MessageEntry,
    raw_proof: RawMerkleProof,
}

impl MessageEntryProof {
    pub fn new(entry: MessageEntry, raw_proof: RawMerkleProof) -> Self {
        Self { entry, raw_proof }
    }

    pub fn entry(&self) -> &MessageEntry {
        &self.entry
    }

    pub fn raw_proof(&self) -> &RawMerkleProof {
        &self.raw_proof
    }
}
