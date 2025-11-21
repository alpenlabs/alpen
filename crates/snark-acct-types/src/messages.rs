//! Snark account types.

use strata_acct_types::{AccountId, BitcoinAmount, MsgPayload, RawMerkleProof};

use crate::ssz_generated::ssz::messages::{MessageEntry, MessageEntryProof};

impl MessageEntry {
    /// Creates a new message entry.
    pub fn new(source: AccountId, incl_epoch: u32, payload: MsgPayload) -> Self {
        Self {
            source,
            incl_epoch,
            payload,
        }
    }

    /// Gets the source account ID.
    pub fn source(&self) -> AccountId {
        self.source
    }

    /// Gets the inclusion epoch.
    pub fn incl_epoch(&self) -> u32 {
        self.incl_epoch
    }

    /// Gets the message payload.
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

impl MessageEntryProof {
    /// Creates a new message entry proof.
    pub fn new(entry: MessageEntry, raw_proof: RawMerkleProof) -> Self {
        Self { entry, raw_proof }
    }

    /// Gets the message entry.
    pub fn entry(&self) -> &MessageEntry {
        &self.entry
    }

    /// Gets the raw merkle proof.
    pub fn raw_proof(&self) -> &RawMerkleProof {
        &self.raw_proof
    }
}
