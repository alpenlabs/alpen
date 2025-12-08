//! Sled-backed MMR Database implementation

use strata_merkle::{
    hasher::MerkleHasher, mmr::CompactMmrData, CompactMmr64B32 as CompactMmr64,
    MerkleProofB32 as MerkleProof, Sha256Hasher,
};

use super::{
    helpers::{
        find_peak_for_pos, get_peaks, leaf_index_to_pos, parent_pos, pos_height_in_tree,
        sibling_pos,
    },
    types::{MmrDatabase, MmrDbError, MmrDbResult},
};

// Hash type for 32-byte hashes
type Hash = [u8; 32];

/// Sled-backed MMR database for persistent proof generation
///
/// This implementation stores all MMR nodes in Sled database,
/// enabling O(log n) proof generation on-the-fly with persistence.
///
/// # Storage
///
/// - **Nodes Tree**: position -> hash (all 2n-1 nodes)
/// - **Metadata Tree**: metadata key -> MmrMetadata
/// - **In-memory cache**: peak_roots, num_leaves, mmr_size
///
/// # Performance
///
/// - Append: O(log n) + Sled writes
/// - Proof generation: O(log n) Sled reads
/// - Storage: ~64 MB for 1M leaves
#[derive(Debug, Clone)]
pub struct SledMmrDb {
    /// Sled tree for MMR nodes (position -> hash)
    nodes_tree: sled::Tree,

    /// Sled tree for metadata
    metadata_tree: sled::Tree,

    /// Cached metadata (synchronized with Sled)
    num_leaves: u64,
    mmr_size: u64,
    peak_roots: Vec<Hash>,
}

impl SledMmrDb {
    /// Create a new SledMmrDb from Sled trees
    pub fn new(nodes_tree: sled::Tree, metadata_tree: sled::Tree) -> MmrDbResult<Self> {
        // Load metadata from storage or initialize
        let (num_leaves, mmr_size, peak_roots) = Self::load_metadata(&metadata_tree)?;

        Ok(Self {
            nodes_tree,
            metadata_tree,
            num_leaves,
            mmr_size,
            peak_roots,
        })
    }

    /// Load metadata from Sled
    fn load_metadata(tree: &sled::Tree) -> MmrDbResult<(u64, u64, Vec<Hash>)> {
        const META_KEY: &[u8] = b"metadata";

        match tree
            .get(META_KEY)
            .map_err(|e| MmrDbError::Storage(e.to_string()))?
        {
            Some(bytes) => {
                // Deserialize metadata using borsh
                let metadata: MmrMetadata = borsh::from_slice(&bytes).map_err(|e| {
                    MmrDbError::Storage(format!("Failed to deserialize metadata: {}", e))
                })?;

                Ok((metadata.num_leaves, metadata.mmr_size, metadata.peak_roots))
            }
            None => {
                // Empty database
                Ok((0, 0, Vec::new()))
            }
        }
    }

    /// Save metadata to Sled
    fn save_metadata(&self) -> MmrDbResult<()> {
        const META_KEY: &[u8] = b"metadata";

        let metadata = MmrMetadata {
            num_leaves: self.num_leaves,
            mmr_size: self.mmr_size,
            peak_roots: self.peak_roots.clone(),
        };

        let bytes = borsh::to_vec(&metadata)
            .map_err(|e| MmrDbError::Storage(format!("Failed to serialize metadata: {}", e)))?;

        self.metadata_tree
            .insert(META_KEY, bytes.as_slice())
            .map_err(|e| MmrDbError::Storage(e.to_string()))?;

        Ok(())
    }

    /// Hash two nodes together to create parent hash
    fn hash_nodes(left: &Hash, right: &Hash) -> Hash {
        Sha256Hasher::hash_node(*left, *right)
    }

    /// Get a node hash by position from Sled
    fn get_node(&self, pos: u64) -> MmrDbResult<Hash> {
        let key = pos.to_be_bytes();

        self.nodes_tree
            .get(key)
            .map_err(|e| MmrDbError::Storage(e.to_string()))?
            .map(|bytes| {
                let mut hash = [0u8; 32];
                hash.copy_from_slice(&bytes);
                hash
            })
            .ok_or_else(|| MmrDbError::Storage(format!("Node not found at position {}", pos)))
    }

    /// Store a node hash at position in Sled
    fn put_node(&self, pos: u64, hash: &Hash) -> MmrDbResult<()> {
        let key = pos.to_be_bytes();

        self.nodes_tree
            .insert(key, hash.as_slice())
            .map_err(|e| MmrDbError::Storage(e.to_string()))?;

        Ok(())
    }

    /// Update peak_roots by extracting current peaks from nodes
    ///
    /// Note: strata-merkle stores peaks in reverse order (right-to-left / by increasing height)
    /// while get_peaks() returns them in left-to-right position order.
    /// We must reverse to match strata-merkle's expected ordering.
    fn update_peak_roots(&mut self) -> MmrDbResult<()> {
        let peak_positions = get_peaks(self.mmr_size);
        let mut peaks: Vec<Hash> = peak_positions
            .iter()
            .map(|&pos| self.get_node(pos))
            .collect::<MmrDbResult<Vec<Hash>>>()?;

        // Reverse to match strata-merkle's ordering (right-to-left / by increasing height)
        peaks.reverse();
        self.peak_roots = peaks;

        Ok(())
    }
}

/// Metadata structure for serialization
#[derive(Debug, Clone, borsh::BorshSerialize, borsh::BorshDeserialize)]
struct MmrMetadata {
    num_leaves: u64,
    mmr_size: u64,
    peak_roots: Vec<Hash>,
}

impl MmrDatabase for SledMmrDb {
    fn append_leaf(&mut self, hash: Hash) -> MmrDbResult<u64> {
        let leaf_index = self.num_leaves;
        let leaf_pos = leaf_index_to_pos(leaf_index);

        // Store the leaf
        self.put_node(leaf_pos, &hash)?;

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
                let sibling_hash = self.get_node(sibling_position)?;

                // Create parent hash (left sibling, right sibling)
                let parent_hash = Self::hash_nodes(&sibling_hash, &current_hash);

                // Store parent
                self.put_node(next_pos, &parent_hash)?;

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
        let leaves_count = self.num_leaves;
        let peak_count = leaves_count.count_ones() as u64;
        self.mmr_size = 2 * leaves_count - peak_count;

        // Update peak_roots
        self.update_peak_roots()?;

        // Save metadata to Sled
        self.save_metadata()?;

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
            let sibling_hash = self.get_node(sib_pos)?;

            cohashes.push(sibling_hash);

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
        // Directly construct CompactMmr64 from our stored peak roots
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

impl CompactMmrData for SledMmrDb {
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

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    fn create_test_db() -> (TempDir, SledMmrDb) {
        let temp_dir = TempDir::new().unwrap();
        let db = sled::open(temp_dir.path()).unwrap();
        let nodes_tree = db.open_tree(b"mmr_nodes").unwrap();
        let metadata_tree = db.open_tree(b"mmr_metadata").unwrap();

        let mmr_db = SledMmrDb::new(nodes_tree, metadata_tree).unwrap();
        (temp_dir, mmr_db)
    }

    #[test]
    fn test_new_db_is_empty() {
        let (_dir, db) = create_test_db();
        assert_eq!(db.num_leaves(), 0);
    }

    #[test]
    fn test_append_and_retrieve() {
        let (_dir, mut db) = create_test_db();

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
    fn test_persistence() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().to_path_buf();

        // Create and populate database
        {
            let db = sled::open(&db_path).unwrap();
            let nodes_tree = db.open_tree(b"mmr_nodes").unwrap();
            let metadata_tree = db.open_tree(b"mmr_metadata").unwrap();

            let mut mmr_db = SledMmrDb::new(nodes_tree, metadata_tree).unwrap();

            for i in 0..10 {
                mmr_db.append_leaf([i as u8; 32]).unwrap();
            }
        }

        // Reopen and verify
        {
            let db = sled::open(&db_path).unwrap();
            let nodes_tree = db.open_tree(b"mmr_nodes").unwrap();
            let metadata_tree = db.open_tree(b"mmr_metadata").unwrap();

            let mmr_db = SledMmrDb::new(nodes_tree, metadata_tree).unwrap();

            assert_eq!(mmr_db.num_leaves(), 10);

            // Verify we can generate proofs
            let proof = mmr_db.generate_proof(5).unwrap();
            assert!(!proof.cohashes().is_empty());
        }
    }

    #[test]
    fn test_proof_generation() {
        use strata_merkle::mmr::verify;

        let (_dir, mut db) = create_test_db();

        // Add 10 leaves
        for i in 0..10 {
            db.append_leaf([i as u8; 32]).unwrap();
        }

        // Verify all proofs
        for i in 0..10 {
            let proof = db.generate_proof(i).unwrap();
            let leaf = [i as u8; 32];
            assert!(
                verify::<_, _, Sha256Hasher>(&db, &proof, &leaf),
                "Proof failed for leaf {}",
                i
            );
        }
    }
}
