//! Sled-backed MMR Database implementation

use parking_lot::Mutex;
use strata_db_types::{DbError, DbResult, traits::MmrDatabase};
use strata_merkle::{
    CompactMmr64B32 as CompactMmr64, MerkleProofB32 as MerkleProof, Sha256Hasher,
    hasher::MerkleHasher, mmr::CompactMmrData,
};

use super::{
    mmr_helpers::{
        find_peak_for_pos, get_peaks, leaf_index_to_pos, parent_pos, pos_height_in_tree,
        sibling_pos,
    },
    schemas::{AsmMmrMetaSchema, AsmMmrNodeSchema, MmrMetadata},
};

// Hash type for 32-byte hashes
type Hash = [u8; 32];

/// Cached MMR metadata for fast access
#[derive(Clone)]
struct MmrCache {
    num_leaves: u64,
    mmr_size: u64,
    peak_roots: Vec<Hash>,
}

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
///
/// # Thread Safety
///
/// Uses interior mutability (Mutex) for cached metadata, allowing `&self` methods
/// while maintaining thread-safe mutable state.
pub struct SledMmrDb {
    /// Typed sled tree for MMR nodes (position -> hash)
    nodes_tree: typed_sled::SledTree<AsmMmrNodeSchema>,

    /// Typed sled tree for metadata
    metadata_tree: typed_sled::SledTree<AsmMmrMetaSchema>,

    /// Cached metadata (synchronized with Sled), wrapped in Mutex for interior mutability
    cache: Mutex<MmrCache>,
}

impl Clone for SledMmrDb {
    fn clone(&self) -> Self {
        let cache = self.cache.lock().clone();
        Self {
            nodes_tree: self.nodes_tree.clone(),
            metadata_tree: self.metadata_tree.clone(),
            cache: Mutex::new(cache),
        }
    }
}

impl SledMmrDb {
    /// Create a new SledMmrDb from typed sled trees
    pub fn new(
        nodes_tree: typed_sled::SledTree<AsmMmrNodeSchema>,
        metadata_tree: typed_sled::SledTree<AsmMmrMetaSchema>,
    ) -> DbResult<Self> {
        // Load metadata from storage or initialize
        let (num_leaves, mmr_size, peak_roots) = Self::load_metadata(&metadata_tree)?;

        Ok(Self {
            nodes_tree,
            metadata_tree,
            cache: Mutex::new(MmrCache {
                num_leaves,
                mmr_size,
                peak_roots,
            }),
        })
    }

    /// Load metadata from typed sled tree
    fn load_metadata(
        tree: &typed_sled::SledTree<AsmMmrMetaSchema>,
    ) -> DbResult<(u64, u64, Vec<Hash>)> {
        match tree.get(&())? {
            Some(metadata) => Ok((metadata.num_leaves, metadata.mmr_size, metadata.peak_roots)),
            None => {
                // Empty database
                Ok((0, 0, Vec::new()))
            }
        }
    }

    /// Save metadata to typed sled tree
    fn save_metadata(&self) -> DbResult<()> {
        let cache = self.cache.lock();
        let metadata = MmrMetadata {
            num_leaves: cache.num_leaves,
            mmr_size: cache.mmr_size,
            peak_roots: cache.peak_roots.clone(),
        };
        drop(cache); // Release lock before DB operation

        self.metadata_tree.insert(&(), &metadata)?;

        Ok(())
    }

    /// Hash two nodes together to create parent hash
    fn hash_nodes(left: &Hash, right: &Hash) -> Hash {
        Sha256Hasher::hash_node(*left, *right)
    }

    /// Get a node hash by position from typed sled tree
    fn get_node(&self, pos: u64) -> DbResult<Hash> {
        self.nodes_tree
            .get(&pos)?
            .ok_or_else(|| DbError::Other(format!("MMR node not found at position {}", pos)))
    }

    /// Store a node hash at position in typed sled tree
    fn put_node(&self, pos: u64, hash: &Hash) -> DbResult<()> {
        self.nodes_tree.insert(&pos, hash)?;
        Ok(())
    }

    /// Update peak_roots by extracting current peaks from nodes
    ///
    /// Note: strata-merkle stores peaks in reverse order (right-to-left / by increasing height)
    /// while get_peaks() returns them in left-to-right position order.
    /// We must reverse to match strata-merkle's expected ordering.
    fn update_peak_roots(&self) -> DbResult<()> {
        let mmr_size = self.cache.lock().mmr_size;
        let peak_positions = get_peaks(mmr_size);
        let mut peaks: Vec<Hash> = peak_positions
            .iter()
            .map(|&pos| self.get_node(pos))
            .collect::<DbResult<Vec<Hash>>>()?;

        // Reverse to match strata-merkle's ordering (right-to-left / by increasing height)
        peaks.reverse();
        self.cache.lock().peak_roots = peaks;

        Ok(())
    }
}

impl std::fmt::Debug for SledMmrDb {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let cache = self.cache.lock();
        f.debug_struct("SledMmrDb")
            .field("num_leaves", &cache.num_leaves)
            .field("mmr_size", &cache.mmr_size)
            .field("peak_roots", &cache.peak_roots)
            .finish_non_exhaustive()
    }
}

impl MmrDatabase for SledMmrDb {
    fn append_leaf(&self, hash: Hash) -> DbResult<u64> {
        let leaf_index = self.cache.lock().num_leaves;
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
        let mut cache = self.cache.lock();
        cache.num_leaves += 1;
        let leaves_count = cache.num_leaves;
        let peak_count = leaves_count.count_ones() as u64;
        cache.mmr_size = 2 * leaves_count - peak_count;
        drop(cache); // Release lock before calling other methods

        // Update peak_roots
        self.update_peak_roots()?;

        // Save metadata to Sled
        self.save_metadata()?;

        Ok(leaf_index)
    }

    fn generate_proof(&self, index: u64) -> DbResult<MerkleProof> {
        // Check bounds and get mmr_size in one lock
        let (num_leaves, mmr_size) = {
            let cache = self.cache.lock();
            (cache.num_leaves, cache.mmr_size)
        };

        if index >= num_leaves {
            return Err(DbError::MmrLeafNotFound(index));
        }

        // Convert leaf index to MMR position
        let leaf_pos = leaf_index_to_pos(index);

        // Find which peak this leaf belongs to
        let peak_pos = find_peak_for_pos(leaf_pos, mmr_size);

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

    fn generate_proofs(&self, start: u64, end: u64) -> DbResult<Vec<MerkleProof>> {
        // Validate range
        if start > end {
            return Err(DbError::MmrInvalidRange { start, end });
        }

        let num_leaves = self.cache.lock().num_leaves;
        if end >= num_leaves {
            return Err(DbError::MmrLeafNotFound(end));
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
        self.cache.lock().num_leaves
    }

    fn peak_roots(&self) -> Vec<Hash> {
        self.cache.lock().peak_roots.clone()
    }

    fn to_compact(&self) -> CompactMmr64 {
        // Directly construct CompactMmr64 from our stored peak roots
        use ssz_types::FixedBytes;

        let cache = self.cache.lock();
        // Convert our Hash to FixedBytes<32> for SSZ
        let roots_vec: Vec<_> = cache
            .peak_roots
            .iter()
            .map(|h| FixedBytes::<32>::from(*h))
            .collect();

        CompactMmr64 {
            entries: cache.num_leaves,
            cap_log2: 64,
            roots: roots_vec.into(),
        }
    }
}

impl CompactMmrData for SledMmrDb {
    type Hash = Hash;

    fn entries(&self) -> u64 {
        self.cache.lock().num_leaves
    }

    fn cap_log2(&self) -> u8 {
        64 // Support up to 2^64 leaves
    }

    fn roots_iter(&self) -> impl Iterator<Item = &Self::Hash> {
        // Note: Due to Mutex-based interior mutability, this implementation is not practical.
        // Tests should use `peak_roots()` directly or verify proofs through `to_compact()`.
        std::iter::empty()
    }

    fn get_root_for_height(&self, height: usize) -> Option<&Self::Hash> {
        // Note: This method cannot work properly with Mutex interior mutability
        // as we cannot return a reference to data inside the Mutex.
        //
        // For testing: Use `strata_merkle::mmr::verify` with a temporary CompactMmr64
        // created via `to_compact()` instead of using the DB directly.
        let _ = height; // silence warning
        None
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    fn create_test_db() -> (TempDir, SledMmrDb) {
        use super::super::schemas::{AsmMmrMetaSchema, AsmMmrNodeSchema};

        let temp_dir = TempDir::new().unwrap();
        let raw_db = sled::open(temp_dir.path()).unwrap();
        let typed_db = typed_sled::SledDb::new(raw_db).unwrap();

        let nodes_tree = typed_db.get_tree::<AsmMmrNodeSchema>().unwrap();
        let metadata_tree = typed_db.get_tree::<AsmMmrMetaSchema>().unwrap();

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
        let (_dir, db) = create_test_db();

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
        use super::super::schemas::{AsmMmrMetaSchema, AsmMmrNodeSchema};

        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().to_path_buf();

        // Create and populate database
        {
            let raw_db = sled::open(&db_path).unwrap();
            let typed_db = typed_sled::SledDb::new(raw_db).unwrap();

            let nodes_tree = typed_db.get_tree::<AsmMmrNodeSchema>().unwrap();
            let metadata_tree = typed_db.get_tree::<AsmMmrMetaSchema>().unwrap();

            let mmr_db = SledMmrDb::new(nodes_tree, metadata_tree).unwrap();

            for i in 0..10 {
                mmr_db.append_leaf([i as u8; 32]).unwrap();
            }
        }

        // Reopen and verify
        {
            let raw_db = sled::open(&db_path).unwrap();
            let typed_db = typed_sled::SledDb::new(raw_db).unwrap();

            let nodes_tree = typed_db.get_tree::<AsmMmrNodeSchema>().unwrap();
            let metadata_tree = typed_db.get_tree::<AsmMmrMetaSchema>().unwrap();

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

        let (_dir, db) = create_test_db();

        // Add 10 leaves
        for i in 0..10 {
            db.append_leaf([i as u8; 32]).unwrap();
        }

        // Get compact representation for verification
        let compact = db.to_compact();

        // Verify all proofs
        for i in 0..10 {
            let proof = db.generate_proof(i).unwrap();
            let leaf = [i as u8; 32];
            assert!(
                verify::<_, _, Sha256Hasher>(&compact, &proof, &leaf),
                "Proof failed for leaf {}",
                i
            );
        }
    }
}
