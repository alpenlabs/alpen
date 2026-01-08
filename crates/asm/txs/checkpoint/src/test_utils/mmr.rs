//! Test MMR helper for checkpoint subprotocol unit tests.
//!
//! Provides a full MMR implementation that stores all nodes in memory,
//! enabling proof generation for unit tests without a database backend.
//!
//! ## Production Semantics
//!
//! This module models the production MMR semantics where:
//! - Genesis block has **NO entry** in the MMR (it's a boundary, not a processable block)
//! - The first MMR leaf (index 0) contains the manifest hash for `genesis_height + 1`
//! - Formula: `mmr_index = l1_height - genesis_height - 1`
//! - Reverse: `l1_height = genesis_height + mmr_index + 1`
//!
//! This matches the behavior in `crates/asm/worker/src/aux_resolver.rs`.

use std::collections::HashMap;

use strata_asm_common::{
    ASM_MMR_CAP_LOG2, AsmCompactMmr, AsmHasher, AsmMerkleProof, AsmMmr, AuxData, Hash32,
    VerifiableManifestHash, VerifiedAuxData,
};
use strata_db_types::mmr_helpers::{
    BitManipulatedMmrAlgorithm, MmrAlgorithm, MmrError as DbMmrError, MmrMetadata,
};
use strata_merkle::MerkleProofB32 as MerkleProof;
use thiserror::Error;

/// Test MMR that stores all nodes in memory for proof generation.
///
/// Unlike the production MMR which only stores peaks (compact form), this stores
/// all nodes in a hashmap to enable proof generation via [`BitManipulatedMmrAlgorithm`].
///
/// Internally maintains both:
/// - A full node store for proof generation
/// - A standard `AsmMmr` for compact MMR conversion
#[derive(Debug, Clone)]
pub struct TestMmr {
    /// All MMR nodes indexed by position.
    nodes: HashMap<u64, Hash32>,
    /// Current MMR metadata (num_leaves, mmr_size, peaks).
    metadata: MmrMetadata,
    /// Standard MMR for compact conversion (only stores peaks).
    standard_mmr: AsmMmr,
}

impl TestMmr {
    /// Creates a new empty test MMR.
    pub fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            metadata: MmrMetadata::empty(),
            standard_mmr: AsmMmr::new(ASM_MMR_CAP_LOG2),
        }
    }

    /// Appends a leaf hash to the MMR.
    ///
    /// Returns the leaf index of the appended leaf.
    ///
    /// # Panics
    ///
    /// Panics if `hash` is the zero hash (`[0u8; 32]`), since strata-merkle
    /// treats this as a sentinel value indicating "no value".
    pub fn append(&mut self, hash: Hash32) -> u64 {
        let result =
            BitManipulatedMmrAlgorithm::append_leaf::<_, MmrError>(hash, &self.metadata, |pos| {
                self.nodes
                    .get(&pos)
                    .copied()
                    .ok_or(MmrError::NodeNotFound(pos))
            })
            .expect("append should succeed with valid metadata");

        // Store all new nodes
        for (pos, node_hash) in result.nodes_to_write {
            self.nodes.insert(pos, node_hash);
        }

        self.metadata = result.new_metadata;

        // Keep standard MMR in sync for compact conversion using add_leaf
        strata_merkle::Mmr::<AsmHasher>::add_leaf(&mut self.standard_mmr, hash)
            .expect("add_leaf should succeed");

        result.leaf_index
    }

    /// Generates an MMR proof for a leaf at the given index.
    pub fn generate_proof(&self, leaf_index: u64) -> AsmMerkleProof {
        let proof = BitManipulatedMmrAlgorithm::generate_proof::<_, MmrError>(
            leaf_index,
            self.metadata.mmr_size,
            |pos| {
                self.nodes
                    .get(&pos)
                    .copied()
                    .ok_or(MmrError::NodeNotFound(pos))
            },
        )
        .expect("proof generation should succeed for valid leaf index");

        // Convert to AsmMerkleProof
        convert_proof(proof)
    }

    /// Returns the current number of leaves in the MMR.
    pub fn num_leaves(&self) -> u64 {
        self.metadata.num_leaves
    }

    /// Converts to a compact MMR for verification.
    pub fn to_compact(&self) -> AsmCompactMmr {
        self.standard_mmr.clone()
    }
}

impl Default for TestMmr {
    fn default() -> Self {
        Self::new()
    }
}

/// Simple error type for MMR operations.
#[derive(Debug, Error)]
enum MmrError {
    #[error("MMR node not found at position {0}")]
    NodeNotFound(u64),
    #[error("MMR algorithm error: {0}")]
    Mmr(#[from] DbMmrError),
}

/// Converts a `MerkleProof<[u8; 32]>` to `AsmMerkleProof`.
fn convert_proof(proof: MerkleProof) -> AsmMerkleProof {
    AsmMerkleProof::from_cohashes(proof.cohashes().to_vec(), proof.index())
}

/// Creates verified auxiliary data for a range of L1 block heights.
///
/// This function models **production MMR semantics** where:
/// - Genesis block has NO entry in the MMR
/// - MMR index 0 = manifest hash for `genesis_height + 1`
/// - Formula: `mmr_index = l1_height - genesis_height - 1`
///
/// # Arguments
///
/// * `genesis_height` - The L1 height of the genesis block (excluded from MMR)
/// * `start_height` - First L1 height to include in aux data (must be > genesis_height)
/// * `end_height` - Last L1 height to include in aux data (inclusive)
///
/// # Returns
///
/// A tuple of:
/// * `VerifiedAuxData` - Verified auxiliary data with manifest hashes for [start, end]
/// * `AsmCompactMmr` - The compact MMR used for verification
///
/// # Panics
///
/// Panics if `start_height <= genesis_height` (first manifest must be after genesis).
pub fn verified_aux_data_with_genesis(
    genesis_height: u64,
    start_height: u64,
    end_height: u64,
) -> (VerifiedAuxData, AsmCompactMmr) {
    assert!(
        start_height > genesis_height,
        "start_height ({start_height}) must be > genesis_height ({genesis_height}); \
         genesis block has no manifest hash in the MMR"
    );
    assert!(
        end_height >= start_height,
        "end_height ({end_height}) must be >= start_height ({start_height})"
    );

    let mut mmr = TestMmr::new();

    // Build MMR from genesis+1 up to end_height.
    // Genesis is NOT in the MMR - the first entry is for genesis+1.
    //
    // MMR layout (production semantics):
    //   Index 0: manifest_hash(genesis + 1)
    //   Index 1: manifest_hash(genesis + 2)
    //   ...
    //   Index N: manifest_hash(genesis + N + 1)
    let first_l1_height = genesis_height + 1;
    for l1_height in first_l1_height..=end_height {
        let hash = manifest_hash_for_l1_height(l1_height);
        let mmr_index = mmr.append(hash);

        // Verify the index matches production formula
        let expected_index = l1_height - genesis_height - 1;
        debug_assert_eq!(
            mmr_index, expected_index,
            "MMR index mismatch: got {mmr_index}, expected {expected_index} for L1 height {l1_height}"
        );
    }

    // Generate proofs only for the requested range [start_height, end_height]
    let mut verifiable_hashes = Vec::with_capacity((end_height - start_height + 1) as usize);
    for l1_height in start_height..=end_height {
        let hash = manifest_hash_for_l1_height(l1_height);

        // Production formula: mmr_index = l1_height - genesis_height - 1
        let mmr_index = l1_height - genesis_height - 1;
        let proof = mmr.generate_proof(mmr_index);

        verifiable_hashes.push(VerifiableManifestHash::new(l1_height, hash, proof));
    }

    let compact_mmr = mmr.to_compact();
    let aux_data = AuxData::new(verifiable_hashes, vec![]);

    let verified = VerifiedAuxData::try_new(&aux_data, &compact_mmr)
        .expect("aux data should verify with matching MMR");

    (verified, compact_mmr)
}

/// Creates verified auxiliary data for a range of L1 block heights.
///
/// Convenience wrapper that uses genesis_height = 0.
/// See [`verified_aux_data_with_genesis`] for full documentation.
///
/// # Arguments
///
/// * `start_height` - First L1 height (must be > 0, since genesis at 0 is excluded)
/// * `end_height` - Last L1 height (inclusive)
///
/// # Panics
///
/// Panics if `start_height == 0` (genesis has no manifest hash).
pub fn verified_aux_data_for_heights(
    start_height: u64,
    end_height: u64,
) -> (VerifiedAuxData, AsmCompactMmr) {
    verified_aux_data_with_genesis(0, start_height, end_height)
}

/// Generates a deterministic manifest hash for a given L1 height.
///
/// Returns a non-zero hash derived from the L1 height for predictable test values.
/// The hash is `[(l1_height % 255 + 1) as u8; 32]` to ensure it's never the zero hash
/// (which strata-merkle treats as a sentinel for "no value").
///
/// # Note
///
/// This function should only be called for L1 heights **after genesis**.
/// Genesis blocks do not have manifest hashes in the production MMR.
pub fn manifest_hash_for_l1_height(l1_height: u64) -> Hash32 {
    // Use modulo to handle large heights while ensuring non-zero
    // The +1 ensures we never return [0u8; 32] even for height 0
    // (though height 0 = genesis shouldn't be called in production semantics)
    [((l1_height % 255) + 1) as u8; 32]
}

#[cfg(test)]
mod tests {
    use strata_asm_common::AsmHasher;

    use super::*;

    #[test]
    fn test_mmr_append_and_proof() {
        let mut mmr = TestMmr::new();

        // NOTE: We use non-zero hashes because [0u8; 32] is treated as the "zero hash"
        // in strata-merkle, which signals "no value" and cannot be used as a valid leaf.
        let leaf0 = [1u8; 32];
        let leaf1 = [2u8; 32];
        let leaf2 = [3u8; 32];

        // Append some leaves
        let idx0 = mmr.append(leaf0);
        let idx1 = mmr.append(leaf1);
        let idx2 = mmr.append(leaf2);

        assert_eq!(idx0, 0);
        assert_eq!(idx1, 1);
        assert_eq!(idx2, 2);
        assert_eq!(mmr.num_leaves(), 3);

        // Generate proofs and verify against compact MMR
        let compact = mmr.to_compact();

        let proof0 = mmr.generate_proof(0);
        let proof1 = mmr.generate_proof(1);
        let proof2 = mmr.generate_proof(2);

        assert!(compact.verify::<AsmHasher>(&proof0, &leaf0));
        assert!(compact.verify::<AsmHasher>(&proof1, &leaf1));
        assert!(compact.verify::<AsmHasher>(&proof2, &leaf2));

        // Wrong hash should not verify
        assert!(!compact.verify::<AsmHasher>(&proof0, &leaf1));
    }

    #[test]
    fn test_production_semantics_genesis_at_zero() {
        // Genesis at height 0, checkpoint covers heights 1-5
        let genesis_height = 0;
        let (verified, _compact) = verified_aux_data_with_genesis(genesis_height, 1, 5);

        // Should be able to get manifest hashes for heights 1-5
        for l1_height in 1..=5 {
            let hash = verified
                .get_manifest_hash(l1_height)
                .expect("hash should exist");
            assert_eq!(hash, manifest_hash_for_l1_height(l1_height));
        }

        // Genesis height (0) should not exist - it's not in the MMR
        assert!(verified.get_manifest_hash(0).is_err());

        // Heights outside the requested range should not exist in aux data
        assert!(verified.get_manifest_hash(6).is_err());
    }

    #[test]
    fn test_production_semantics_genesis_at_100() {
        // Genesis at height 100, checkpoint covers heights 101-105
        let genesis_height = 100;
        let (verified, _compact) = verified_aux_data_with_genesis(genesis_height, 101, 105);

        // Should be able to get manifest hashes for heights 101-105
        for l1_height in 101..=105 {
            let hash = verified
                .get_manifest_hash(l1_height)
                .expect("hash should exist");
            assert_eq!(hash, manifest_hash_for_l1_height(l1_height));
        }

        // Genesis height (100) should not exist
        assert!(verified.get_manifest_hash(100).is_err());
    }

    #[test]
    fn test_mmr_index_formula() {
        // Verify the production formula: mmr_index = l1_height - genesis_height - 1
        let genesis_height = 50;
        let (verified, _compact) = verified_aux_data_with_genesis(genesis_height, 51, 55);

        // Manually verify the formula
        // L1 height 51 -> index = 51 - 50 - 1 = 0
        // L1 height 52 -> index = 52 - 50 - 1 = 1
        // L1 height 55 -> index = 55 - 50 - 1 = 4
        for l1_height in 51..=55 {
            let expected_index = l1_height - genesis_height - 1;
            let hash = verified
                .get_manifest_hash(l1_height)
                .expect("hash should exist");

            // The hash should match what we'd compute for this L1 height
            assert_eq!(
                hash,
                manifest_hash_for_l1_height(l1_height),
                "Hash mismatch at L1 height {l1_height} (expected MMR index {expected_index})"
            );
        }
    }

    #[test]
    fn test_convenience_wrapper() {
        // The convenience wrapper uses genesis_height = 0
        let (verified, _compact) = verified_aux_data_for_heights(1, 3);

        for l1_height in 1..=3 {
            let hash = verified
                .get_manifest_hash(l1_height)
                .expect("hash should exist");
            assert_eq!(hash, manifest_hash_for_l1_height(l1_height));
        }

        // Genesis (height 0) should not be in aux data
        assert!(verified.get_manifest_hash(0).is_err());
    }

    #[test]
    #[should_panic(expected = "start_height (0) must be > genesis_height (0)")]
    fn test_start_at_genesis_panics() {
        // Starting at genesis height should panic - genesis has no manifest hash
        let _ = verified_aux_data_for_heights(0, 3);
    }

    #[test]
    #[should_panic(expected = "start_height (100) must be > genesis_height (100)")]
    fn test_start_at_genesis_with_explicit_genesis_panics() {
        // Starting at genesis height should panic
        let _ = verified_aux_data_with_genesis(100, 100, 105);
    }

    #[test]
    fn test_manifest_hash_never_zero() {
        // Verify that manifest_hash_for_l1_height never returns zero hash
        for l1_height in 0..1000 {
            let hash = manifest_hash_for_l1_height(l1_height);
            assert_ne!(
                hash, [0u8; 32],
                "Hash for height {l1_height} must not be zero"
            );
        }
    }
}
