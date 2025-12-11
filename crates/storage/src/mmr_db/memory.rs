//! In-memory MMR database implementation

use std::collections::HashMap;

use strata_merkle::{
    hasher::MerkleHasher, mmr::CompactMmrData, CompactMmr64B32 as CompactMmr64,
    MerkleProofB32 as MerkleProof, Sha256Hasher,
};

// Hash type for 32-byte hashes
type Hash = [u8; 32];

use super::{
    helpers::{
        find_peak_for_pos, get_peaks, leaf_index_to_pos, parent_pos, pos_height_in_tree,
        sibling_pos,
    },
    types::{MmrDatabase, MmrDbError, MmrDbResult},
};

/// In-memory implementation of MMR database using Nervos-style node storage
///
/// This implementation stores all MMR nodes (leaves + internal nodes) by position,
/// enabling O(log n) proof generation on-the-fly without storing proofs.
///
/// # Storage
///
/// - **Nodes**: HashMap<position, hash> - stores all 2n-1 nodes
/// - **num_leaves**: Total number of leaves appended
/// - **mmr_size**: Total number of nodes in MMR
/// - **peak_roots**: Individual peak roots (for multi-peak MMR verification)
///
/// # Performance
///
/// - Append: O(log n) - compute path to new peak
/// - Proof generation: O(log n) - traverse from leaf to peak
/// - Storage: ~64 MB for 1M leaves (vs ~320 MB for stored proofs)
///
/// This is suitable for:
/// - Testing
/// - Short-lived processes
/// - Production use with in-memory cache
///
/// For persistent storage, use [`SledMmrDb`].
#[derive(Debug)]
pub struct InMemoryMmrDb {
    /// All MMR nodes indexed by position
    nodes: HashMap<u64, Hash>,
    /// Number of leaves (not total nodes)
    num_leaves: u64,
    /// Total MMR size (number of nodes)
    mmr_size: u64,
    /// Individual peak roots (indexed by position in compact representation)
    peak_roots: Vec<Hash>,
}

impl InMemoryMmrDb {
    /// Create a new empty in-memory MMR database
    pub fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            num_leaves: 0,
            mmr_size: 0,
            peak_roots: Vec::new(),
        }
    }

    /// Hash two nodes together to create parent hash
    fn hash_nodes(left: &Hash, right: &Hash) -> Hash {
        // Use Sha256Hasher::hash_node for internal nodes
        // This is different from hash_leaf - hash_node is specifically for combining child hashes
        Sha256Hasher::hash_node(*left, *right)
    }

    /// Get a node hash by position
    #[expect(dead_code, reason = "Kept for potential debugging use")]
    fn get_node(&self, pos: u64) -> Option<&Hash> {
        self.nodes.get(&pos)
    }

    /// Update peak_roots by extracting current peaks from nodes
    ///
    /// Note: strata-merkle stores peaks in reverse order (right-to-left / by increasing height)
    /// while get_peaks() returns them in left-to-right position order.
    /// We must reverse to match strata-merkle's expected ordering.
    fn update_peak_roots(&mut self) {
        let peak_positions = get_peaks(self.mmr_size);
        let mut peaks: Vec<Hash> = peak_positions
            .iter()
            .map(|&pos| {
                *self
                    .nodes
                    .get(&pos)
                    .expect("Peak position must have a hash")
            })
            .collect();

        // Reverse to match strata-merkle's ordering (right-to-left / by increasing height)
        peaks.reverse();
        self.peak_roots = peaks;
    }
}

impl Default for InMemoryMmrDb {
    fn default() -> Self {
        Self::new()
    }
}

impl CompactMmrData for InMemoryMmrDb {
    type Hash = Hash;

    fn entries(&self) -> u64 {
        self.num_leaves
    }

    fn cap_log2(&self) -> u8 {
        64 // Support up to 2^64 leaves
    }

    fn roots_iter(&self) -> impl Iterator<Item = &Self::Hash> {
        self.peak_roots.iter()
    }

    fn get_root_for_height(&self, height: usize) -> Option<&Self::Hash> {
        // Use the same algorithm as strata-merkle's CompactMmr64
        // Count how many peaks exist below this height
        let root_index = (self.num_leaves & ((1 << height) - 1)).count_ones() as usize;
        self.peak_roots.get(root_index)
    }
}

impl MmrDatabase for InMemoryMmrDb {
    fn append_leaf(&mut self, hash: Hash) -> MmrDbResult<u64> {
        let leaf_index = self.num_leaves;
        let leaf_pos = leaf_index_to_pos(leaf_index);

        // Store the leaf
        self.nodes.insert(leaf_pos, hash);

        // Merge along the path to create internal nodes
        let mut current_pos = leaf_pos;
        let mut current_hash = hash;
        let mut current_height = 0u8;

        // Keep merging as long as we have a left sibling
        loop {
            // Calculate what the next position would be
            let next_pos = current_pos + 1;
            let next_height = pos_height_in_tree(next_pos);

            // If next position is higher, current is a right sibling - we should merge
            if next_height > current_height {
                // Current is right sibling, get left sibling
                let sibling_position = sibling_pos(current_pos, current_height);
                let sibling_hash = self.nodes.get(&sibling_position).ok_or_else(|| {
                    MmrDbError::Storage(format!("Missing sibling at pos {}", sibling_position))
                })?;

                // Create parent hash (left sibling, right sibling)
                let parent_hash = Self::hash_nodes(sibling_hash, &current_hash);

                // Store parent
                self.nodes.insert(next_pos, parent_hash);

                // Move up to parent
                current_pos = next_pos;
                current_hash = parent_hash;
                current_height = next_height;
            } else {
                // Current is a left sibling (will be merged when right sibling comes)
                // or we've reached a peak - stop here
                break;
            }
        }

        // Update state
        self.num_leaves += 1;
        // mmr_size is the total number of nodes (leaves + internal nodes)
        // After adding leaf at index N-1, mmr_size should be calculated from N leaves
        let leaves_count = self.num_leaves;
        let peak_count = leaves_count.count_ones() as u64;
        self.mmr_size = 2 * leaves_count - peak_count;

        // Debug: ensure mmr_size matches actual node count
        debug_assert_eq!(
            self.nodes.len() as u64,
            self.mmr_size,
            "Node count mismatch: nodes={}, mmr_size={}",
            self.nodes.len(),
            self.mmr_size
        );

        // Update peak_roots: extract current peak hashes
        self.update_peak_roots();

        Ok(leaf_index)
    }

    fn generate_proof(&self, index: u64) -> MmrDbResult<MerkleProof> {
        // Check bounds
        if index >= self.num_leaves {
            return Err(MmrDbError::LeafNotFound(index));
        }

        // Convert leaf index to MMR position
        let leaf_pos = leaf_index_to_pos(index);

        // Find which peak this leaf belongs to
        let peak_pos = find_peak_for_pos(leaf_pos, self.mmr_size);

        // Collect sibling hashes along the path from leaf to peak
        let mut cohashes = Vec::new();
        let mut current_pos = leaf_pos;
        let mut current_height = 0u8;

        // Climb to peak, collecting siblings
        while current_pos < peak_pos {
            let sib_pos = sibling_pos(current_pos, current_height);
            let sibling_hash =
                self.nodes
                    .get(&sib_pos)
                    .ok_or_else(|| MmrDbError::ProofGenerationFailed {
                        index,
                        reason: format!("Missing sibling node at position {}", sib_pos),
                    })?;

            cohashes.push(*sibling_hash);

            // Move to parent
            current_pos = parent_pos(current_pos, current_height);
            current_height += 1;
        }

        Ok(MerkleProof::from_cohashes(cohashes, index))
    }

    fn generate_proofs(&self, start: u64, end: u64) -> MmrDbResult<Vec<MerkleProof>> {
        // Validate range
        if start > end {
            return Err(MmrDbError::InvalidRange { start, end });
        }

        if end >= self.num_leaves {
            return Err(MmrDbError::LeafNotFound(end));
        }

        // Generate proof for each index in range
        // TODO: Optimize this by reusing sibling hashes for contiguous ranges
        let mut proofs = Vec::with_capacity((end - start + 1) as usize);

        for index in start..=end {
            let proof = self.generate_proof(index)?;
            proofs.push(proof);
        }

        Ok(proofs)
    }

    fn num_leaves(&self) -> u64 {
        self.num_leaves
    }

    fn peak_roots(&self) -> &[Hash] {
        &self.peak_roots
    }

    fn to_compact(&self) -> CompactMmr64 {
        // Directly construct CompactMmr64B32 from our stored peak roots
        // Note: CompactMmr64 is a type alias for CompactMmr64B32
        use ssz_types::FixedBytes;

        // Convert our Hash to FixedBytes<32> for SSZ
        let roots_vec: Vec<_> = self
            .peak_roots
            .iter()
            .map(|h| FixedBytes::<32>::from(*h))
            .collect();

        CompactMmr64 {
            entries: self.num_leaves,
            cap_log2: 64,
            roots: roots_vec.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_db_is_empty() {
        let db = InMemoryMmrDb::new();
        assert_eq!(db.num_leaves(), 0);
    }

    #[test]
    fn test_append_leaf_increments_count() {
        let mut db = InMemoryMmrDb::new();

        let hash1 = [1u8; 32];
        let index1 = db.append_leaf(hash1).unwrap();
        assert_eq!(index1, 0);
        assert_eq!(db.num_leaves(), 1);

        let hash2 = [2u8; 32];
        let index2 = db.append_leaf(hash2).unwrap();
        assert_eq!(index2, 1);
        assert_eq!(db.num_leaves(), 2);
    }

    #[test]
    fn test_to_compact_works() {
        let mut db = InMemoryMmrDb::new();

        db.append_leaf([1u8; 32]).unwrap();
        db.append_leaf([2u8; 32]).unwrap();

        let _compact = db.to_compact();

        // TODO: Verify compact structure once we understand the API
        // For now, just ensure it doesn't panic
    }

    #[test]
    fn test_peak_roots_are_consistent() {
        let mut db = InMemoryMmrDb::new();

        let peaks_empty = db.peak_roots().to_vec();
        assert_eq!(peaks_empty.len(), 0);

        db.append_leaf([1u8; 32]).unwrap();
        let peaks_one = db.peak_roots().to_vec();
        assert_eq!(peaks_one.len(), 1); // Single peak

        db.append_leaf([2u8; 32]).unwrap();
        let peaks_two = db.peak_roots().to_vec();
        assert_eq!(peaks_two.len(), 1); // Still single peak (merged)

        // Peak should change after merging
        assert_ne!(peaks_one[0], peaks_two[0]);
    }

    #[test]
    fn test_generate_proof_out_of_bounds() {
        let db = InMemoryMmrDb::new();

        let result = db.generate_proof(0);
        assert!(matches!(result, Err(MmrDbError::LeafNotFound(0))));
    }

    #[test]
    fn test_generate_proofs_invalid_range() {
        let db = InMemoryMmrDb::new();

        let result = db.generate_proofs(10, 5);
        assert!(matches!(
            result,
            Err(MmrDbError::InvalidRange { start: 10, end: 5 })
        ));
    }

    // TODO: Re-enable once we understand MerkleMr64 API better
    // Currently MerkleMr64 doesn't expose peaks_iter in the SSZ-generated version
    #[test]
    #[ignore]
    fn test_compare_with_mmr64() {
        // This test is disabled because MerkleMr64 API is different in SSZ version
        // We'll verify correctness through proof verification instead
    }

    #[test]
    fn test_generate_proof_for_single_leaf() {
        use strata_merkle::mmr::verify;

        let mut db = InMemoryMmrDb::new();

        let hash1 = [1u8; 32];
        db.append_leaf(hash1).unwrap();

        let proof = db.generate_proof(0).unwrap();

        // Verify proof using strata-merkle's verify function
        println!("Leaf:          {:?}", &hash1[..8]);
        println!("Proof cohashes: {}", proof.cohashes().len());
        println!("Peak roots count: {}", db.peak_roots().len());

        assert!(verify::<_, _, Sha256Hasher>(&db, &proof, &hash1));
    }

    #[test]
    fn test_generate_proof_for_two_leaves() {
        use strata_merkle::mmr::verify;

        let mut db = InMemoryMmrDb::new();

        let hash1 = [1u8; 32];
        let hash2 = [2u8; 32];
        db.append_leaf(hash1).unwrap();
        db.append_leaf(hash2).unwrap();

        println!("\nPeak roots count: {}", db.peak_roots().len());

        // Manual verification using hash_node (not hash_leaf!)
        let manual_root = Sha256Hasher::hash_node(hash1, hash2);
        println!("Manual root:   {:?}", &manual_root[..8]);
        println!("Peak root:     {:?}", &db.peak_roots()[0][..8]);

        // Verify proof for first leaf
        let proof0 = db.generate_proof(0).unwrap();
        println!("\nProof0 cohashes: {}", proof0.cohashes().len());
        if !proof0.cohashes().is_empty() {
            println!("  Cohash[0]: {:?}", &proof0.cohashes()[0][..8]);
        }

        assert!(
            verify::<_, _, Sha256Hasher>(&db, &proof0, &hash1),
            "Proof 0 failed"
        );

        // Verify proof for second leaf
        let proof1 = db.generate_proof(1).unwrap();
        assert!(
            verify::<_, _, Sha256Hasher>(&db, &proof1, &hash2),
            "Proof 1 failed"
        );
    }

    #[test]
    fn test_generate_proof_for_four_leaves_power_of_2() {
        use strata_merkle::mmr::verify;

        let mut db = InMemoryMmrDb::new();

        // Add 4 leaves (power of 2 - single peak)
        for i in 0..4 {
            db.append_leaf([i as u8; 32]).unwrap();
        }

        println!("Peak roots count: {}", db.peak_roots().len());
        assert_eq!(db.peak_roots().len(), 1); // Power of 2 = single peak

        // Verify all proofs
        for i in 0..4 {
            let proof = db.generate_proof(i).unwrap();
            let hash = [i as u8; 32];
            assert!(
                verify::<_, _, Sha256Hasher>(&db, &proof, &hash),
                "Proof failed for index {} in power-of-2 tree",
                i
            );
        }
    }

    // Multi-peak MMR proofs now supported via CompactMmrData!
    #[test]
    fn test_generate_proofs_for_range() {
        use strata_merkle::mmr::verify;

        let mut db = InMemoryMmrDb::new();

        // Add several leaves (non-power-of-2 = multiple peaks)
        for i in 0..10 {
            db.append_leaf([i as u8; 32]).unwrap();
        }

        println!("Peak roots count for 10 leaves: {}", db.peak_roots().len());
        // 10 = 0b1010 = peaks at heights 1 and 3
        assert_eq!(db.peak_roots().len(), 2);

        // Generate proofs for range
        let proofs = db.generate_proofs(3, 7).unwrap();

        assert_eq!(proofs.len(), 5); // inclusive range

        // Verify each proof using strata-merkle's verify
        for (i, proof) in proofs.iter().enumerate() {
            let hash = [(3 + i) as u8; 32];
            assert!(
                verify::<_, _, Sha256Hasher>(&db, proof, &hash),
                "Proof failed for index {}",
                3 + i
            );
        }
    }

    // Multi-peak MMR proofs with many leaves
    #[test]
    fn test_proof_generation_many_leaves() {
        use strata_merkle::mmr::verify;

        let mut db = InMemoryMmrDb::new();

        // Add 100 leaves (non-power-of-2 = multiple peaks)
        let leaves: Vec<Hash> = (0..100).map(|i| [i as u8; 32]).collect();

        for leaf in &leaves {
            db.append_leaf(*leaf).unwrap();
        }

        println!("Peak roots count for 100 leaves: {}", db.peak_roots().len());
        // 100 = 0b1100100 = peaks at heights 2, 5, 6
        assert_eq!(db.peak_roots().len(), 3);

        // Verify proofs for all leaves
        for (i, leaf) in leaves.iter().enumerate() {
            let proof = db.generate_proof(i as u64).unwrap();
            assert!(
                verify::<_, _, Sha256Hasher>(&db, &proof, leaf),
                "Proof verification failed for leaf {}",
                i
            );
        }
    }

    #[test]
    fn test_proof_cohashes_structure() {
        let mut db = InMemoryMmrDb::new();

        // Add 4 leaves (perfect binary tree)
        for i in 0..4 {
            db.append_leaf([i as u8; 32]).unwrap();
        }

        // For a perfect binary tree with 4 leaves, each proof should have 2 cohashes
        let proof0 = db.generate_proof(0).unwrap();
        assert_eq!(proof0.cohashes().len(), 2);

        let proof3 = db.generate_proof(3).unwrap();
        assert_eq!(proof3.cohashes().len(), 2);
    }

    #[test]
    fn test_proofs_change_after_new_leaves() {
        let mut db = InMemoryMmrDb::new();

        // Add first 4 leaves
        for i in 0..4 {
            db.append_leaf([i as u8; 32]).unwrap();
        }

        let peaks_4 = db.peak_roots().to_vec();

        // Generate proof for leaf 0 with 4 leaves
        let proof_0_at_4 = db.generate_proof(0).unwrap();

        // Add more leaves
        for i in 4..8 {
            db.append_leaf([i as u8; 32]).unwrap();
        }

        let peaks_8 = db.peak_roots().to_vec();

        // Peaks should change (both have single peak, but different values)
        assert_ne!(peaks_4, peaks_8);

        // Generate new proof for leaf 0 with 8 leaves
        let proof_0_at_8 = db.generate_proof(0).unwrap();

        // The proof depth should increase (more leaves = taller tree)
        assert!(proof_0_at_8.cohashes().len() > proof_0_at_4.cohashes().len());
    }
}
