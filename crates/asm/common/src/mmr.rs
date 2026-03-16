//! History accumulator for ASM.

use strata_asm_manifest_types::{AsmManifest, Hash32};
use strata_merkle::{CompactMmr64, Mmr, Sha256Hasher, error::MerkleError};

use crate::AsmHistoryAccumulatorState;

/// Capacity of the ASM MMR as a power of 2.
///
/// With a value of 64, the MMR supports up to 2^64 leaves
const ASM_MMR_CAP_LOG2: u8 = 64;

/// The hasher used for ASM manifest MMR operations.
///
/// Uses SHA-256 with full 32-byte hash output.
pub type AsmHasher = Sha256Hasher;

pub type AsmMerkleProof = strata_acct_types::MerkleProof;

impl AsmHistoryAccumulatorState {
    /// Creates a new compact MMR for the given genesis height.
    ///
    /// The internal `offset` is set to `genesis_height + 1` since manifests
    /// start from the first block after genesis.
    pub fn new(genesis_height: u64) -> Self {
        let manifest_mmr =
            strata_acct_types::Mmr64::from_generic(&CompactMmr64::<Hash32>::new(ASM_MMR_CAP_LOG2));
        Self {
            manifest_mmr,
            offset: genesis_height + 1,
        }
    }

    /// Returns the height offset for MMR index-to-height conversion.
    pub fn offset(&self) -> u64 {
        self.offset
    }

    /// Returns the current number of leaves in the manifest MMR.
    pub fn num_entries(&self) -> u64 {
        self.manifest_mmr.num_entries()
    }

    /// Returns the L1 block height of the last manifest inserted into the MMR.
    ///
    /// Returns the genesis height if the MMR is empty.
    pub fn last_inserted_height(&self) -> u64 {
        // offset + num_entries - 1 because num_entries() is the count but MMR indices start at 0
        self.offset + self.manifest_mmr.num_entries() - 1
    }

    /// Verifies a Merkle proof for a leaf in the MMR.
    pub fn verify_manifest_leaf(&self, proof: &AsmMerkleProof, leaf: &Hash32) -> bool {
        self.manifest_mmr.verify(proof, leaf)
    }

    /// Adds a new leaf to the MMR.
    pub fn add_manifest_leaf(&mut self, leaf: Hash32) -> Result<(), MerkleError> {
        Mmr::<AsmHasher>::add_leaf(&mut self.manifest_mmr, leaf)
    }

    pub fn verify_manifest(&mut self, proof: &AsmMerkleProof, manifest: AsmManifest) -> bool {
        let leaf_hash = manifest.compute_hash();
        self.verify_manifest_leaf(proof, &leaf_hash)
    }

    pub fn add_manifest(&mut self, manifest: &AsmManifest) -> Result<(), MerkleError> {
        let leaf_hash = manifest.compute_hash();
        self.add_manifest_leaf(leaf_hash)
    }
}
