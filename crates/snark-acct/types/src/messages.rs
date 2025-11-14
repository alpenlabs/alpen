//! Snark account types.

use strata_acct_types::{AccountId, BitcoinAmount, MsgPayload, RawMerkleProof};
use strata_primitives::Epoch;

/// Message entry in an account inbox.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MessageEntry {
    /// The source account ID of the message.
    source: AccountId,

    /// The epoch that the message was included.
    incl_epoch: Epoch,

    /// The message payload.
    payload: MsgPayload,
}

impl MessageEntry {
    pub fn new(source: AccountId, incl_epoch: Epoch, payload: MsgPayload) -> Self {
        Self {
            source,
            incl_epoch,
            payload,
        }
    }

    pub fn source(&self) -> AccountId {
        self.source
    }

    pub fn incl_epoch(&self) -> Epoch {
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

    // FIXME: just a placeholder until ssz
    // Simple serialization for testing - NOT the real SSZ format
    pub fn to_ssz_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        // Serialize source account ID (32 bytes)
        bytes.extend_from_slice(self.source.inner());
        // Serialize epoch (8 bytes)
        bytes.extend_from_slice(&self.incl_epoch.to_le_bytes());
        // Serialize payload value (8 bytes)
        bytes.extend_from_slice(&self.payload.value().to_sat().to_le_bytes());
        // Serialize payload data length (8 bytes)
        let data_len = self.payload.data().len() as u64;
        bytes.extend_from_slice(&data_len.to_le_bytes());
        // Serialize payload data
        bytes.extend_from_slice(self.payload.data());
        bytes
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
