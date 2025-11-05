//! Auxiliary data response types.
//!
//! Defines the structures that workers use to fulfill auxiliary data requests.

use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_common::AsmManifestHash;

use crate::types::ManifestMmrProof;

/// A manifest leaf from the MMR.
///
/// The leaf itself is just the `AsmManifestHash` with its MMR proof.
/// This is lightweight - it doesn't contain the full manifest data
/// (blkid, wtxids_root, logs), only the commitment hash and proof.
///
/// Use this when you need to verify that specific blocks were processed
/// by the ASM, but don't need access to the detailed log data.
#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct ManifestLeaf {
    /// L1 block height for this manifest
    pub height: u64,

    /// The manifest hash (this is the actual leaf value in the MMR).
    ///
    /// Computed as: `Hash(AsmManifest)` where the manifest contains
    /// blkid, wtxids_root, and logs.
    pub manifest_hash: AsmManifestHash,

    /// MMR proof showing that `manifest_hash` is committed in the MMR.
    pub mmr_proof: ManifestMmrProof,
}

impl ManifestLeaf {
    /// Creates a new manifest leaf.
    pub fn new(height: u64, manifest_hash: AsmManifestHash, mmr_proof: ManifestMmrProof) -> Self {
        Self {
            height,
            manifest_hash,
            mmr_proof,
        }
    }

    /// Returns the manifest hash (the MMR leaf value).
    pub fn hash(&self) -> &AsmManifestHash {
        &self.manifest_hash
    }

    /// Returns the MMR proof.
    pub fn proof(&self) -> &ManifestMmrProof {
        &self.mmr_proof
    }

    /// Returns the L1 block height.
    pub fn height(&self) -> u64 {
        self.height
    }
}

/// Auxiliary data response fulfilling a specific request.
///
/// Workers create these envelopes to provide the data requested by
/// subprotocols during pre-processing. Each envelope corresponds to
/// one `AuxRequestSpec`.
#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub enum AuxResponseEnvelope {
    /// Manifest leaves (hash + proof) for a requested block range.
    ///
    /// This is the response to `AuxRequestSpec::ManifestLeaves`.
    /// Contains one `ManifestLeaf` per block in the requested range.
    ManifestLeaves {
        /// Starting L1 block height (inclusive)
        start_height: u64,
        /// Ending L1 block height (inclusive)
        end_height: u64,
        /// One leaf per block in range, ordered by height
        leaves: Vec<ManifestLeaf>,
    },

    /// Raw Bitcoin transaction bytes.
    ///
    /// This is the response to `AuxRequestSpec::BitcoinTx`.
    /// Contains the full serialized transaction data.
    ///
    /// Note: Txid is stored as 32 bytes since bitcoin::Txid doesn't implement Borsh traits.
    BitcoinTx {
        /// The transaction ID (32 bytes)
        txid: [u8; 32],
        /// Raw transaction bytes
        raw_tx: Vec<u8>,
    },
}

impl AuxResponseEnvelope {
    /// Returns the variant name as a string (for error messages).
    pub fn variant_name(&self) -> &'static str {
        match self {
            Self::ManifestLeaves { .. } => "ManifestLeaves",
            Self::BitcoinTx { .. } => "BitcoinTx",
        }
    }

    /// Creates a manifest leaves response.
    pub fn manifest_leaves(
        start_height: u64,
        end_height: u64,
        leaves: Vec<ManifestLeaf>,
    ) -> Self {
        Self::ManifestLeaves {
            start_height,
            end_height,
            leaves,
        }
    }

    /// Creates a Bitcoin transaction response.
    ///
    /// # Arguments
    /// * `txid` - The transaction ID (32 bytes)
    /// * `raw_tx` - Raw transaction bytes
    pub fn bitcoin_tx(txid: [u8; 32], raw_tx: Vec<u8>) -> Self {
        Self::BitcoinTx { txid, raw_tx }
    }
}
