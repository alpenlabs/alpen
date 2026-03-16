//! Auxiliary request and response data.
//!
//! Defines the SSZ-backed types used to request and return auxiliary data for
//! ASM processing.

use bitcoin::{
    Transaction, Txid,
    consensus::{deserialize, encode::Error as ConsensusEncodeError},
    hashes::Hash,
};
use ssz_types::FixedBytes;
use strata_asm_manifest_types::Hash32;

use crate::{
    AsmMerkleProof, AuxData, AuxRequests, BitcoinTxid, ManifestHashRange, RawBitcoinTx,
    VerifiableManifestHash,
};

impl AuxRequests {
    /// Returns a slice of the requested manifest hash ranges.
    pub fn manifest_hashes(&self) -> &[ManifestHashRange] {
        &self.manifest_hashes
    }

    /// Returns a slice of the requested Bitcoin transaction IDs.
    pub fn bitcoin_txs(&self) -> &[BitcoinTxid] {
        &self.bitcoin_txs
    }
}

impl Default for AuxRequests {
    fn default() -> Self {
        Self {
            manifest_hashes: Vec::new().into(),
            bitcoin_txs: Vec::new().into(),
        }
    }
}

impl AuxData {
    /// Creates a new auxiliary data collection.
    pub fn new(
        manifest_hashes: Vec<VerifiableManifestHash>,
        bitcoin_txs: Vec<RawBitcoinTx>,
    ) -> Self {
        Self {
            manifest_hashes: manifest_hashes.into(),
            bitcoin_txs: bitcoin_txs.into(),
        }
    }

    /// Returns a slice of manifest hashes with their MMR proofs.
    pub fn manifest_hashes(&self) -> &[VerifiableManifestHash] {
        &self.manifest_hashes
    }

    /// Returns a slice of raw Bitcoin transactions.
    pub fn bitcoin_txs(&self) -> &[RawBitcoinTx] {
        &self.bitcoin_txs
    }
}

impl Default for AuxData {
    fn default() -> Self {
        Self::new(vec![], vec![])
    }
}

impl ManifestHashRange {
    /// Creates a new manifest hash range.
    pub fn new(start_height: u64, end_height: u64) -> Self {
        Self {
            start_height,
            end_height,
        }
    }

    /// Returns the start height (inclusive).
    pub fn start_height(&self) -> u64 {
        self.start_height
    }

    /// Returns the end height (inclusive).
    pub fn end_height(&self) -> u64 {
        self.end_height
    }
}

impl VerifiableManifestHash {
    /// Creates a new verifiable manifest hash.
    pub fn new(hash: Hash32, proof: AsmMerkleProof) -> Self {
        Self {
            hash: FixedBytes::from(hash),
            proof,
        }
    }

    /// Returns the manifest hash.
    pub fn hash(&self) -> Hash32 {
        self.hash
            .as_ref()
            .try_into()
            .expect("asm: fixed bytes hash must be 32 bytes")
    }

    /// Returns a reference to the MMR proof.
    pub fn proof(&self) -> &AsmMerkleProof {
        &self.proof
    }
}

impl crate::BitcoinTxid {
    /// Creates an ASM-local txid from the native Bitcoin txid wrapper.
    pub fn from_native(txid: strata_btc_types::BitcoinTxid) -> Self {
        Self {
            bytes: FixedBytes::from(txid.inner().to_byte_array()),
        }
    }

    /// Converts the ASM-local txid back into the native Bitcoin txid wrapper.
    pub fn into_native(self) -> strata_btc_types::BitcoinTxid {
        let bytes: [u8; 32] = self
            .bytes
            .as_ref()
            .try_into()
            .expect("asm: txid bytes must be 32 bytes");
        Txid::from_byte_array(bytes).into()
    }
}

impl From<Txid> for crate::BitcoinTxid {
    fn from(value: Txid) -> Self {
        strata_btc_types::BitcoinTxid::new(&value).into()
    }
}

impl From<strata_btc_types::BitcoinTxid> for crate::BitcoinTxid {
    fn from(value: strata_btc_types::BitcoinTxid) -> Self {
        Self::from_native(value)
    }
}

impl From<crate::BitcoinTxid> for strata_btc_types::BitcoinTxid {
    fn from(value: crate::BitcoinTxid) -> Self {
        value.into_native()
    }
}

impl crate::RawBitcoinTx {
    /// Creates an ASM-local raw Bitcoin transaction from raw bytes.
    pub fn from_raw_bytes(bytes: Vec<u8>) -> Self {
        Self {
            bytes: bytes.into(),
        }
    }

    /// Returns the raw transaction bytes.
    pub fn as_raw_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Consumes the wrapper and returns the raw bytes.
    pub fn into_raw_bytes(self) -> Vec<u8> {
        self.bytes.iter().copied().collect()
    }

    /// Creates an ASM-local raw Bitcoin transaction from the native wrapper.
    pub fn from_native(raw_tx: strata_btc_types::RawBitcoinTx) -> Self {
        Self::from_raw_bytes(raw_tx.into_raw_bytes())
    }

    /// Converts the ASM-local raw Bitcoin transaction back into the native wrapper.
    pub fn into_native(self) -> strata_btc_types::RawBitcoinTx {
        strata_btc_types::RawBitcoinTx::from_raw_bytes(self.into_raw_bytes())
    }
}

impl From<strata_btc_types::RawBitcoinTx> for crate::RawBitcoinTx {
    fn from(value: strata_btc_types::RawBitcoinTx) -> Self {
        Self::from_native(value)
    }
}

impl From<crate::RawBitcoinTx> for strata_btc_types::RawBitcoinTx {
    fn from(value: crate::RawBitcoinTx) -> Self {
        value.into_native()
    }
}

impl TryFrom<&crate::RawBitcoinTx> for Transaction {
    type Error = ConsensusEncodeError;

    fn try_from(value: &crate::RawBitcoinTx) -> Result<Self, Self::Error> {
        deserialize(value.as_raw_bytes())
    }
}

impl TryFrom<crate::RawBitcoinTx> for Transaction {
    type Error = ConsensusEncodeError;

    fn try_from(value: crate::RawBitcoinTx) -> Result<Self, Self::Error> {
        deserialize(value.as_raw_bytes())
    }
}
