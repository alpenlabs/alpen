//! MMR types for ASM manifests.

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use strata_asm_manifest_types::Hash32;
use strata_merkle::{CompactMmr64, MerkleProof, Mmr, Sha256Hasher, error::MerkleError};

/// Capacity of the ASM MMR as a power of 2.
///
/// With a value of 64, the MMR supports up to 2^64 leaves
const ASM_MMR_CAP_LOG2: u8 = 64;

/// The hasher used for ASM manifest MMR operations.
///
/// Uses SHA-256 with full 32-byte hash output.
pub type AsmHasher = Sha256Hasher;

pub type AsmMerkleProof = MerkleProof<Hash32>;

/// Compact Merkle Mountain Range for ASM manifest hashes.
///
/// This structure maintains a compact MMR of manifest hashes with a height offset for
/// mapping MMR indices to L1 block heights.
///
/// # Example
///
/// ```ignore
/// // L2 starts anchoring to L1 at Bitcoin block 800000
/// let mmr = AsmManifestMmr::new(800000);
/// // offset will be 800001 internally
/// mmr.add_leaf(hash1)?; // MMR index 0, L1 block height 800001
/// mmr.add_leaf(hash2)?; // MMR index 1, L1 block height 800002
/// ```
#[derive(Clone, Debug, PartialEq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct AsmManifestMmr {
    mmr: CompactMmr64<Hash32>,
    /// Height offset for mapping MMR indices to L1 block heights.
    /// Equal to `genesis_height + 1` since manifests start after genesis.
    offset: u64,
}

impl AsmManifestMmr {
    /// Creates a new compact MMR for the given genesis height.
    ///
    /// The internal `offset` is set to `genesis_height + 1` since manifests
    /// start from the first block after genesis.
    pub fn new(genesis_height: u64) -> Self {
        let mmr = CompactMmr64::new(ASM_MMR_CAP_LOG2);
        Self {
            mmr,
            offset: genesis_height + 1,
        }
    }

    /// Returns the height offset for MMR index-to-height conversion.
    pub fn offset(&self) -> u64 {
        self.offset
    }

    /// Verifies a Merkle proof for a leaf in the MMR.
    pub fn verify(&self, proof: &AsmMerkleProof, leaf: &Hash32) -> bool {
        self.mmr.verify::<AsmHasher>(proof, leaf)
    }

    /// Adds a new leaf to the MMR.
    pub fn add_leaf(&mut self, leaf: Hash32) -> Result<(), MerkleError> {
        Mmr::<AsmHasher>::add_leaf(&mut self.mmr, leaf)
    }
}
