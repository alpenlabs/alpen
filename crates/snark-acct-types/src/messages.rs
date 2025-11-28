//! Snark account types.

use borsh::{BorshDeserialize, BorshSerialize};
use ssz::{Decode, Encode};
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

// Implement BorshSerialize/BorshDeserialize for database storage
// These types are SSZ types, but we add Borsh implementations for database storage
impl BorshSerialize for MessageEntry {
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        // Serialize as SSZ bytes, then write length + bytes
        let ssz_bytes = self.as_ssz_bytes();
        let len = ssz_bytes.len() as u32;
        borsh::BorshSerialize::serialize(&len, writer)?;
        writer.write_all(&ssz_bytes)?;
        Ok(())
    }
}

impl BorshDeserialize for MessageEntry {
    fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> std::io::Result<Self> {
        // Read length, then SSZ bytes
        let len = u32::deserialize_reader(reader)?;
        let mut ssz_bytes = vec![0u8; len as usize];
        reader.read_exact(&mut ssz_bytes)?;
        MessageEntry::from_ssz_bytes(&ssz_bytes).map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("Failed to decode MessageEntry from SSZ: {:?}", e),
            )
        })
    }
}

impl BorshSerialize for MessageEntryProof {
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        // Serialize as SSZ bytes, then write length + bytes
        let ssz_bytes = self.as_ssz_bytes();
        let len = ssz_bytes.len() as u32;
        borsh::BorshSerialize::serialize(&len, writer)?;
        writer.write_all(&ssz_bytes)?;
        Ok(())
    }
}

impl BorshDeserialize for MessageEntryProof {
    fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> std::io::Result<Self> {
        // Read length, then SSZ bytes
        let len = u32::deserialize_reader(reader)?;
        let mut ssz_bytes = vec![0u8; len as usize];
        reader.read_exact(&mut ssz_bytes)?;
        MessageEntryProof::from_ssz_bytes(&ssz_bytes).map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("Failed to decode MessageEntryProof from SSZ: {:?}", e),
            )
        })
    }
}
