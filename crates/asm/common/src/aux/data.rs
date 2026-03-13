//! Auxiliary request and response data.
//!
//! Defines the types of auxiliary data that subprotocols can request during
//! the pre-processing phase, along with the response structures returned
//! to subprotocols after verification.

use ssz::{Decode, DecodeError, Encode};
use ssz_derive::{Decode as DeriveDecode, Encode as DeriveEncode};
use strata_asm_manifest_types::Hash32;
use strata_btc_types::{BitcoinTxid, RawBitcoinTx};
use strata_merkle::MerkleProofB32;

use crate::AsmMerkleProof;

/// Collection of auxiliary data requests from subprotocols.
///
/// During pre-processing, subprotocols declare what auxiliary data they need.
/// External workers fulfill that before the main processing phase.
#[derive(Debug, Clone, Default, DeriveEncode, DeriveDecode)]
pub struct AuxRequests {
    /// Requested manifest hash height ranges.
    pub(crate) manifest_hashes: Vec<ManifestHashRange>,

    /// [Txid](bitcoin::Txid) of the requested transactions.
    pub(crate) bitcoin_txs: Vec<BitcoinTxid>,
}

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

/// Collection of auxiliary data responses for subprotocols.
///
/// Contains unverified Bitcoin transactions and manifest hashes returned by external workers.
/// This data must be validated before use during the main processing phase.
#[derive(Debug, Clone, Default, PartialEq, DeriveEncode, DeriveDecode)]
pub struct AuxData {
    /// Manifest hashes with their MMR proofs (unverified)
    manifest_hashes: Vec<VerifiableManifestHash>,
    /// Raw Bitcoin transaction data (unverified)
    bitcoin_txs: Vec<RawBitcoinTx>,
}

impl AuxData {
    /// Creates a new auxiliary data collection.
    pub fn new(
        manifest_hashes: Vec<VerifiableManifestHash>,
        bitcoin_txs: Vec<RawBitcoinTx>,
    ) -> Self {
        Self {
            manifest_hashes,
            bitcoin_txs,
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

/// Manifest hash height range (inclusive).
///
/// Represents a range of L1 block heights for which manifest hashes are requested.
#[derive(Debug, Clone, Copy, DeriveEncode, DeriveDecode)]
pub struct ManifestHashRange {
    /// Start height (inclusive)
    pub(crate) start_height: u64,
    /// End height (inclusive)
    pub(crate) end_height: u64,
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

/// Manifest hash with its MMR proof.
///
/// Contains a hash of an [`AsmManifest`](crate::AsmManifest) along with an MMR proof
/// that can be used to verify the hash's inclusion in the manifest MMR at a specific position.
///
/// This is unverified data - the proof must be verified against a trusted compact MMR
/// before the hash can be considered valid.
#[derive(Debug, Clone, PartialEq)]
pub struct VerifiableManifestHash {
    /// The hash of an [`AsmManifest`](crate::AsmManifest)
    hash: Hash32,
    /// The MMR proof for this manifest hash
    proof: AsmMerkleProof,
}

#[derive(DeriveEncode, DeriveDecode)]
struct VerifiableManifestHashSsz {
    hash: Hash32,
    proof: MerkleProofB32,
}

impl VerifiableManifestHash {
    /// Creates a new verifiable manifest hash.
    pub fn new(hash: Hash32, proof: AsmMerkleProof) -> Self {
        Self { hash, proof }
    }

    /// Returns the manifest hash.
    pub fn hash(&self) -> &Hash32 {
        &self.hash
    }

    /// Returns a reference to the MMR proof.
    pub fn proof(&self) -> &AsmMerkleProof {
        &self.proof
    }
}

impl Encode for VerifiableManifestHash {
    fn is_ssz_fixed_len() -> bool {
        <VerifiableManifestHashSsz as Encode>::is_ssz_fixed_len()
    }

    fn ssz_fixed_len() -> usize {
        <VerifiableManifestHashSsz as Encode>::ssz_fixed_len()
    }

    fn ssz_append(&self, buf: &mut Vec<u8>) {
        VerifiableManifestHashSsz {
            hash: self.hash,
            proof: MerkleProofB32::from_generic(&self.proof),
        }
        .ssz_append(buf);
    }

    fn ssz_bytes_len(&self) -> usize {
        VerifiableManifestHashSsz {
            hash: self.hash,
            proof: MerkleProofB32::from_generic(&self.proof),
        }
        .ssz_bytes_len()
    }
}

impl Decode for VerifiableManifestHash {
    fn is_ssz_fixed_len() -> bool {
        <VerifiableManifestHashSsz as Decode>::is_ssz_fixed_len()
    }

    fn ssz_fixed_len() -> usize {
        <VerifiableManifestHashSsz as Decode>::ssz_fixed_len()
    }

    fn from_ssz_bytes(bytes: &[u8]) -> Result<Self, DecodeError> {
        let value = VerifiableManifestHashSsz::from_ssz_bytes(bytes)?;
        Ok(Self {
            hash: value.hash,
            proof: AsmMerkleProof::from_cohashes(value.proof.cohashes(), value.proof.index()),
        })
    }
}
