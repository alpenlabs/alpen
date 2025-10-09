use std::io::{Read, Write};

use arbitrary::{Arbitrary, Unstructured};
use bitcoin::{hashes::Hash, Txid};
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use strata_identifiers::Buf32;

use crate::constants::HASH_SIZE;

/// [Borsh](borsh)-friendly Bitcoin [`Txid`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BitcoinTxid(Txid);

impl From<Txid> for BitcoinTxid {
    fn from(value: Txid) -> Self {
        Self(value)
    }
}

impl From<BitcoinTxid> for Txid {
    fn from(value: BitcoinTxid) -> Self {
        value.0
    }
}

impl BitcoinTxid {
    /// Creates a new [`BitcoinTxid`] from a [`Txid`].
    ///
    /// # Notes
    ///
    /// [`Txid`] is [`Copy`].
    pub fn new(txid: &Txid) -> Self {
        BitcoinTxid(*txid)
    }

    /// Gets the inner Bitcoin [`Txid`]
    pub fn inner(&self) -> Txid {
        self.0
    }

    /// Gets the inner Bitcoin [`Txid`] as raw bytes [`Buf32`].
    pub fn inner_raw(&self) -> Buf32 {
        self.0.to_byte_array().into()
    }
}

impl BorshSerialize for BitcoinTxid {
    fn serialize<W: Write>(&self, writer: &mut W) -> std::io::Result<()> {
        // Serialize the txid using bitcoin's built-in serialization
        let txid_bytes = self.0.to_byte_array();
        // First, write the length of the serialized txid (as u32)
        BorshSerialize::serialize(&(32_u32), writer)?;
        // Then, write the actual serialized PSBT bytes
        writer.write_all(&txid_bytes)?;
        Ok(())
    }
}

impl BorshDeserialize for BitcoinTxid {
    fn deserialize_reader<R: Read>(reader: &mut R) -> std::io::Result<Self> {
        // First, read the length tag
        let len = u32::deserialize_reader(reader)? as usize;

        if len != HASH_SIZE {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("Invalid Txid size, expected: {HASH_SIZE}, got: {len}"),
            ));
        }

        // First, create a buffer to hold the txid bytes and read them
        let mut txid_bytes = [0u8; HASH_SIZE];
        reader.read_exact(&mut txid_bytes)?;
        // Use the bitcoin crate's deserialize method to create a Psbt from the bytes
        let txid = Txid::from_byte_array(txid_bytes);
        Ok(BitcoinTxid(txid))
    }
}

impl<'a> Arbitrary<'a> for BitcoinTxid {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        let value = Buf32::arbitrary(u)?;
        Ok(Self(Txid::from_byte_array(value.into())))
    }
}
