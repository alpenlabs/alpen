//! Test MMR helper for checkpoint subprotocol unit tests.
//!
//! Provides a full MMR implementation that stores all nodes in memory,
//! enabling proof generation for unit tests without a database backend.

use std::collections::HashMap;

use strata_asm_common::{
    AsmCompactMmr, AsmMerkleProof, AsmMmr, AuxData, Hash32, VerifiableManifestHash, VerifiedAuxData,
    ASM_MMR_CAP_LOG2,
};
use strata_db_types::mmr_helpers::{
    BitManipulatedMmrAlgorithm, MmrAlgorithm, MmrError as DbMmrError, MmrMetadata,
};
use thiserror::Error;
use strata_merkle::MerkleProofB32 as MerkleProof;

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
        self.standard_mmr
            .add_leaf(hash)
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
        self.standard_mmr.clone().into()
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
/// Builds a test MMR with manifest hashes for each height in the range,
/// generates proofs, and returns properly verified `VerifiedAuxData`.
///
/// The manifest hash for each height is `[height as u8; 32]` for deterministic testing.
///
/// # Arguments
///
/// * `start_height` - First L1 height (inclusive)
/// * `end_height` - Last L1 height (inclusive)
///
/// # Returns
///
/// A tuple of:
/// * `VerifiedAuxData` - Verified auxiliary data with manifest hashes
/// * `AsmCompactMmr` - The compact MMR used for verification (for further testing if needed)
pub fn verified_aux_data_for_heights(
    start_height: u64,
    end_height: u64,
) -> (VerifiedAuxData, AsmCompactMmr) {
    // Build MMR with manifest hashes for all heights
    let mut mmr = TestMmr::new();

    // Append placeholder hashes for heights before start_height (if start > 0)
    // The MMR index must match the L1 height for proper verification
    for height in 0..start_height {
        let hash = manifest_hash_for_height(height);
        mmr.append(hash);
    }

    // Append manifest hashes for the requested range
    for height in start_height..=end_height {
        let hash = manifest_hash_for_height(height);
        mmr.append(hash);
    }

    // Generate proofs for the requested range
    let mut verifiable_hashes = Vec::with_capacity((end_height - start_height + 1) as usize);
    for height in start_height..=end_height {
        let hash = manifest_hash_for_height(height);
        let proof = mmr.generate_proof(height);
        verifiable_hashes.push(VerifiableManifestHash::new(height, hash, proof));
    }

    let compact_mmr = mmr.to_compact();
    let aux_data = AuxData::new(verifiable_hashes, vec![]);

    let verified = VerifiedAuxData::try_new(&aux_data, &compact_mmr)
        .expect("aux data should verify with matching MMR");

    (verified, compact_mmr)
}

/// Generates a deterministic manifest hash for a given height.
///
/// Returns `[height as u8; 32]` for predictable test values.
fn manifest_hash_for_height(height: u64) -> Hash32 {
    [height as u8; 32]
}

#[cfg(test)]
mod tests {
    use strata_asm_common::AsmHasher;

    use super::*;

    #[test]
    fn test_mmr_append_and_proof() {
        let mut mmr = TestMmr::new();

        // Append some leaves
        let idx0 = mmr.append([0u8; 32]);
        let idx1 = mmr.append([1u8; 32]);
        let idx2 = mmr.append([2u8; 32]);

        assert_eq!(idx0, 0);
        assert_eq!(idx1, 1);
        assert_eq!(idx2, 2);
        assert_eq!(mmr.num_leaves(), 3);

        // Generate proofs and verify against compact MMR
        let compact = mmr.to_compact();

        let proof0 = mmr.generate_proof(0);
        let proof1 = mmr.generate_proof(1);
        let proof2 = mmr.generate_proof(2);

        assert!(compact.verify::<AsmHasher>(&proof0, &[0u8; 32]));
        assert!(compact.verify::<AsmHasher>(&proof1, &[1u8; 32]));
        assert!(compact.verify::<AsmHasher>(&proof2, &[2u8; 32]));

        // Wrong hash should not verify
        assert!(!compact.verify::<AsmHasher>(&proof0, &[1u8; 32]));
    }

    #[test]
    fn test_verified_aux_data_for_heights() {
        let (verified, _compact) = verified_aux_data_for_heights(5, 10);

        // Should be able to get manifest hashes for heights 5-10
        for height in 5..=10 {
            let hash = verified
                .get_manifest_hash(height)
                .expect("hash should exist");
            assert_eq!(hash, [height as u8; 32]);
        }

        // Heights outside the range should not exist
        assert!(verified.get_manifest_hash(4).is_err());
        assert!(verified.get_manifest_hash(11).is_err());
    }

    #[test]
    fn test_verified_aux_data_from_zero() {
        let (verified, _compact) = verified_aux_data_for_heights(0, 3);

        for height in 0..=3 {
            let hash = verified
                .get_manifest_hash(height)
                .expect("hash should exist");
            assert_eq!(hash, [height as u8; 32]);
        }
    }
}
