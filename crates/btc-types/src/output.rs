use std::io::{self, Read, Write};

use arbitrary::{Arbitrary, Unstructured};
use bitcoin::{hashes::Hash, OutPoint, Txid};
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};

use crate::constants::HASH_SIZE;

/// L1 output reference.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub struct OutputRef(pub OutPoint);

impl From<OutPoint> for OutputRef {
    fn from(value: OutPoint) -> Self {
        Self(value)
    }
}

impl OutputRef {
    pub fn new(txid: Txid, vout: u32) -> Self {
        Self(OutPoint::new(txid, vout))
    }

    pub fn outpoint(&self) -> &OutPoint {
        &self.0
    }
}

// Implement BorshSerialize for the OutputRef wrapper.
impl BorshSerialize for OutputRef {
    fn serialize<W: Write>(&self, writer: &mut W) -> Result<(), io::Error> {
        // Serialize the transaction ID as bytes
        writer.write_all(&self.0.txid[..])?;

        // Serialize the output index as a little-endian 4-byte integer
        writer.write_all(&self.0.vout.to_le_bytes())?;
        Ok(())
    }
}

// Implement BorshDeserialize for the OutputRef wrapper.
impl BorshDeserialize for OutputRef {
    fn deserialize_reader<R: Read>(reader: &mut R) -> Result<Self, io::Error> {
        // Read 32 bytes for the transaction ID
        let mut txid_bytes = [0u8; HASH_SIZE];
        reader.read_exact(&mut txid_bytes)?;
        let txid = bitcoin::Txid::from_slice(&txid_bytes).expect("should be a valid txid (hash)");

        // Read 4 bytes for the output index
        let mut vout_bytes = [0u8; 4];
        reader.read_exact(&mut vout_bytes)?;
        let vout = u32::from_le_bytes(vout_bytes);

        Ok(OutputRef(OutPoint { txid, vout }))
    }
}

// Implement Arbitrary for the wrapper
impl<'a> Arbitrary<'a> for OutputRef {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        // Generate a random 32-byte array for the transaction ID (txid)
        let mut txid_bytes = [0u8; HASH_SIZE];
        u.fill_buffer(&mut txid_bytes)?;
        let txid = bitcoin::Txid::from_raw_hash(bitcoin::hashes::Hash::from_byte_array(txid_bytes));

        // Generate a random 4-byte integer for the output index (vout)
        let vout = u.int_in_range(0..=u32::MAX)?;

        Ok(OutputRef(OutPoint { txid, vout }))
    }
}
