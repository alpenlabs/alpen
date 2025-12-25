use ssz_types::FixedBytes;
use strata_db_types::{
    mmr_helpers::{MmrAlgorithm, MmrId, MmrMetadata},
    DbError, DbResult,
};
use strata_merkle::CompactMmr64B32 as CompactMmr64;
use strata_primitives::buf::Buf32;

use super::schemas::{UnifiedMmrHashIndexSchema, UnifiedMmrMetaSchema, UnifiedMmrNodeSchema};
use crate::define_sled_database;

define_sled_database!(
    pub struct UnifiedMmrDb {
        node_tree: UnifiedMmrNodeSchema,
        meta_tree: UnifiedMmrMetaSchema,
        hash_index_tree: UnifiedMmrHashIndexSchema,
    }
);

impl UnifiedMmrDb {
    fn ensure_mmr_metadata(&self, mmr_id: &MmrId) -> DbResult<()> {
        if self.meta_tree.get(mmr_id)?.is_none() {
            let metadata = MmrMetadata::empty();
            self.meta_tree.insert(mmr_id, &metadata)?;
        }
        Ok(())
    }

    fn load_mmr_metadata(&self, mmr_id: &MmrId) -> DbResult<MmrMetadata> {
        self.meta_tree.get(mmr_id)?.ok_or_else(|| {
            DbError::Other(format!("MMR metadata not found for mmr_id {:?}", mmr_id))
        })
    }

    fn get_mmr_node(&self, mmr_id: &MmrId, pos: u64) -> DbResult<[u8; 32]> {
        self.node_tree
            .get(&(mmr_id.clone(), pos))?
            .map(|buf| buf.0)
            .ok_or_else(|| DbError::MmrNodeNotFound(pos, mmr_id.clone()))
    }

    /// Append a new leaf to the MMR
    pub fn append_leaf(&self, mmr_id: MmrId, hash: [u8; 32]) -> DbResult<u64> {
        self.ensure_mmr_metadata(&mmr_id)?;

        self.config.with_retry(
            (&self.node_tree, &self.meta_tree, &self.hash_index_tree),
            |(nt, mt, hit)| {
                let metadata = mt
                    .get(&mmr_id)?
                    .expect("MMR metadata must exist after ensure_mmr_metadata");

                let result = MmrAlgorithm::append_leaf(hash, &metadata, |pos| {
                    nt.get(&(mmr_id.clone(), pos))?
                        .map(|buf| buf.0)
                        .ok_or_else(|| DbError::MmrNodeNotFound(pos, mmr_id.clone()))
                })
                .map_err(typed_sled::error::Error::abort)?;

                for (pos, node_hash) in &result.nodes_to_write {
                    nt.insert(&(mmr_id.clone(), *pos), &Buf32(*node_hash))?;
                }

                mt.insert(&mmr_id, &result.new_metadata)?;

                // Store hash -> position mapping for the leaf
                hit.insert(&(mmr_id.clone(), Buf32(hash)), &result.leaf_index)?;

                Ok(result.leaf_index)
            },
        )
    }

    /// Get a node at a specific position
    pub fn get_node(&self, mmr_id: MmrId, pos: u64) -> DbResult<[u8; 32]> {
        self.get_mmr_node(&mmr_id, pos)
    }

    /// Get the total MMR size (number of nodes)
    pub fn mmr_size(&self, mmr_id: MmrId) -> DbResult<u64> {
        self.ensure_mmr_metadata(&mmr_id)?;
        let metadata = self.load_mmr_metadata(&mmr_id)?;
        Ok(metadata.mmr_size)
    }

    /// Get the number of leaves in the MMR
    pub fn num_leaves(&self, mmr_id: MmrId) -> DbResult<u64> {
        self.ensure_mmr_metadata(&mmr_id)?;
        let metadata = self.load_mmr_metadata(&mmr_id)?;
        Ok(metadata.num_leaves)
    }

    /// Get the peak roots of the MMR
    pub fn peak_roots(&self, mmr_id: MmrId) -> Vec<[u8; 32]> {
        self.load_mmr_metadata(&mmr_id)
            .map(|m| m.peak_roots.into_iter().map(|b| b.0).collect())
            .unwrap_or_default()
    }

    /// Convert the MMR to compact representation
    pub fn to_compact(&self, mmr_id: MmrId) -> CompactMmr64 {
        let metadata = self
            .load_mmr_metadata(&mmr_id)
            .unwrap_or_else(|_| MmrMetadata::empty());

        let roots_vec: Vec<_> = metadata
            .peak_roots
            .iter()
            .map(|buf| FixedBytes::<32>::from(buf.0))
            .collect();

        CompactMmr64 {
            entries: metadata.num_leaves,
            cap_log2: 64,
            roots: roots_vec.into(),
        }
    }

    /// Remove and return the last leaf from the MMR
    pub fn pop_leaf(&self, mmr_id: MmrId) -> DbResult<Option<[u8; 32]>> {
        self.ensure_mmr_metadata(&mmr_id)?;

        self.config.with_retry(
            (&self.node_tree, &self.meta_tree, &self.hash_index_tree),
            |(nt, mt, hit)| {
                let metadata = mt
                    .get(&mmr_id)?
                    .expect("MMR metadata must exist after ensure_mmr_metadata");

                let result = MmrAlgorithm::pop_leaf(&metadata, |pos| {
                    nt.get(&(mmr_id.clone(), pos))?
                        .map(|buf| buf.0)
                        .ok_or_else(|| DbError::MmrNodeNotFound(pos, mmr_id.clone()))
                })
                .map_err(typed_sled::error::Error::abort)?;

                let Some(result) = result else {
                    return Ok(None);
                };

                for pos in &result.nodes_to_remove {
                    nt.remove(&(mmr_id.clone(), *pos))?;
                }

                mt.insert(&mmr_id, &result.new_metadata)?;

                // Remove hash -> position mapping for the popped leaf
                hit.remove(&(mmr_id.clone(), Buf32(result.leaf_hash)))?;

                Ok(Some(result.leaf_hash))
            },
        )
    }

    /// Get the position of a leaf by its hash (reverse lookup)
    pub fn get_leaf_position(&self, mmr_id: &MmrId, hash: [u8; 32]) -> DbResult<Option<u64>> {
        Ok(self.hash_index_tree.get(&(mmr_id.clone(), Buf32(hash)))?)
    }
}

#[cfg(feature = "test_utils")]
#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use strata_identifiers::AccountId;

    use super::*;
    use crate::test_utils::{get_test_sled_config, get_test_sled_db};

    fn setup() -> UnifiedMmrDb {
        let db = Arc::new(get_test_sled_db());
        let config = get_test_sled_config();
        UnifiedMmrDb::new(db, config).unwrap()
    }

    #[test]
    fn test_append_single_leaf() {
        let db = setup();
        let mmr_id = MmrId::Asm;
        let hash = [1u8; 32];

        // Initially empty
        assert_eq!(db.num_leaves(mmr_id.clone()).unwrap(), 0);
        assert_eq!(db.mmr_size(mmr_id.clone()).unwrap(), 0);

        // Append first leaf
        let idx = db.append_leaf(mmr_id.clone(), hash).unwrap();
        assert_eq!(idx, 0);
        assert_eq!(db.num_leaves(mmr_id.clone()).unwrap(), 1);
        assert_eq!(db.mmr_size(mmr_id.clone()).unwrap(), 1);

        // Can retrieve the node
        let node = db.get_node(mmr_id, 0).unwrap();
        assert_eq!(node, hash);
    }

    #[test]
    fn test_append_multiple_leaves() {
        let db = setup();
        let mmr_id = MmrId::Asm;

        // Append 7 leaves to create a complete tree
        let hashes: Vec<[u8; 32]> = (0..7).map(|i| [i; 32]).collect();

        for (expected_idx, hash) in hashes.iter().enumerate() {
            let idx = db.append_leaf(mmr_id.clone(), *hash).unwrap();
            assert_eq!(idx as usize, expected_idx);
        }

        assert_eq!(db.num_leaves(mmr_id.clone()).unwrap(), 7);
        // 7 leaves with 3 peaks -> mmr_size = 2*7 - 3 = 11
        assert_eq!(db.mmr_size(mmr_id).unwrap(), 11);
    }

    #[test]
    fn test_get_node_positions() {
        let db = setup();
        let mmr_id = MmrId::Asm;

        // Append 4 leaves
        let hashes: Vec<[u8; 32]> = (0..4).map(|i| [i; 32]).collect();

        for hash in &hashes {
            db.append_leaf(mmr_id.clone(), *hash).unwrap();
        }

        // MMR with 4 leaves has 7 nodes:
        // Position: [0, 1, 2, 3, 4, 5, 6]
        // Height:   [0, 0, 1, 0, 0, 1, 2]
        // Leaves:   [0, 1, x, 2, 3, x, x]

        // Verify leaf positions
        assert_eq!(db.get_node(mmr_id.clone(), 0).unwrap(), [0u8; 32]);
        assert_eq!(db.get_node(mmr_id.clone(), 1).unwrap(), [1u8; 32]);
        assert_eq!(db.get_node(mmr_id.clone(), 3).unwrap(), [2u8; 32]);
        assert_eq!(db.get_node(mmr_id.clone(), 4).unwrap(), [3u8; 32]);

        // Internal nodes exist
        assert!(db.get_node(mmr_id.clone(), 2).is_ok());
        assert!(db.get_node(mmr_id.clone(), 5).is_ok());
        assert!(db.get_node(mmr_id, 6).is_ok());
    }

    #[test]
    fn test_peak_roots() {
        let db = setup();
        let mmr_id = MmrId::Asm;

        // Single leaf: one peak
        db.append_leaf(mmr_id.clone(), [1u8; 32]).unwrap();
        let peaks = db.peak_roots(mmr_id.clone());
        assert_eq!(peaks.len(), 1);

        // Two leaves: one peak (merged)
        db.append_leaf(mmr_id.clone(), [2u8; 32]).unwrap();
        let peaks = db.peak_roots(mmr_id.clone());
        assert_eq!(peaks.len(), 1);

        // Three leaves: two peaks
        db.append_leaf(mmr_id.clone(), [3u8; 32]).unwrap();
        let peaks = db.peak_roots(mmr_id.clone());
        assert_eq!(peaks.len(), 2);

        // Four leaves: one peak (complete tree)
        db.append_leaf(mmr_id.clone(), [4u8; 32]).unwrap();
        let peaks = db.peak_roots(mmr_id);
        assert_eq!(peaks.len(), 1);
    }

    #[test]
    fn test_pop_leaf_empty() {
        let db = setup();
        let mmr_id = MmrId::Asm;

        // Popping from empty MMR returns None
        let result = db.pop_leaf(mmr_id).unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn test_pop_leaf_single() {
        let db = setup();
        let mmr_id = MmrId::Asm;

        let hash = [42u8; 32];
        db.append_leaf(mmr_id.clone(), hash).unwrap();

        assert_eq!(db.num_leaves(mmr_id.clone()).unwrap(), 1);

        let popped = db.pop_leaf(mmr_id.clone()).unwrap();
        assert_eq!(popped, Some(hash));
        assert_eq!(db.num_leaves(mmr_id.clone()).unwrap(), 0);
        assert_eq!(db.mmr_size(mmr_id).unwrap(), 0);
    }

    #[test]
    fn test_pop_leaf_multiple() {
        let db = setup();
        let mmr_id = MmrId::Asm;

        // Append several leaves
        let hashes: Vec<[u8; 32]> = (0..5).map(|i| [i; 32]).collect();
        for hash in &hashes {
            db.append_leaf(mmr_id.clone(), *hash).unwrap();
        }

        assert_eq!(db.num_leaves(mmr_id.clone()).unwrap(), 5);

        // Pop last leaf
        let popped = db.pop_leaf(mmr_id.clone()).unwrap();
        assert_eq!(popped, Some([4u8; 32]));
        assert_eq!(db.num_leaves(mmr_id.clone()).unwrap(), 4);

        // Pop another
        let popped = db.pop_leaf(mmr_id.clone()).unwrap();
        assert_eq!(popped, Some([3u8; 32]));
        assert_eq!(db.num_leaves(mmr_id).unwrap(), 3);
    }

    #[test]
    fn test_append_after_pop() {
        let db = setup();
        let mmr_id = MmrId::Asm;

        // Append 3 leaves
        db.append_leaf(mmr_id.clone(), [1u8; 32]).unwrap();
        db.append_leaf(mmr_id.clone(), [2u8; 32]).unwrap();
        db.append_leaf(mmr_id.clone(), [3u8; 32]).unwrap();

        // Pop one
        db.pop_leaf(mmr_id.clone()).unwrap();
        assert_eq!(db.num_leaves(mmr_id.clone()).unwrap(), 2);

        // Append again
        let idx = db.append_leaf(mmr_id.clone(), [4u8; 32]).unwrap();
        assert_eq!(idx, 2);
        assert_eq!(db.num_leaves(mmr_id).unwrap(), 3);
    }

    #[test]
    fn test_to_compact() {
        let db = setup();
        let mmr_id = MmrId::Asm;

        // Empty MMR
        let compact = db.to_compact(mmr_id.clone());
        assert_eq!(compact.entries, 0);

        // Add some leaves
        for i in 0..4 {
            db.append_leaf(mmr_id.clone(), [i; 32]).unwrap();
        }

        let compact = db.to_compact(mmr_id);
        assert_eq!(compact.entries, 4);
        assert_eq!(compact.cap_log2, 64);
        assert!(!compact.roots.is_empty());
    }

    #[test]
    fn test_mmr_size_formula() {
        let db = setup();
        let mmr_id = MmrId::Asm;

        // Test the MMR size formula: size = 2 * leaves - peaks
        // where peaks = number of set bits in binary representation of leaves

        let test_cases = vec![
            (1, 1),  // 1 leaf, 1 peak -> 2*1 - 1 = 1
            (2, 3),  // 2 leaves, 1 peak -> 2*2 - 1 = 3
            (3, 4),  // 3 leaves, 2 peaks -> 2*3 - 2 = 4
            (4, 7),  // 4 leaves, 1 peak -> 2*4 - 1 = 7
            (7, 11), // 7 leaves, 3 peaks -> 2*7 - 3 = 11
        ];

        for (num_leaves, expected_size) in test_cases {
            // Clear and rebuild
            while db.num_leaves(mmr_id.clone()).unwrap() > 0 {
                db.pop_leaf(mmr_id.clone()).unwrap();
            }

            for i in 0..num_leaves {
                db.append_leaf(mmr_id.clone(), [i; 32]).unwrap();
            }

            assert_eq!(
                db.mmr_size(mmr_id.clone()).unwrap(),
                expected_size,
                "MMR size mismatch for {} leaves",
                num_leaves
            );
        }
    }

    #[test]
    fn test_hash_index_lookup() {
        let db = setup();
        let mmr_id = MmrId::Asm;

        let hash1 = [1u8; 32];
        let hash2 = [2u8; 32];

        // Append leaves
        let idx1 = db.append_leaf(mmr_id.clone(), hash1).unwrap();
        let idx2 = db.append_leaf(mmr_id.clone(), hash2).unwrap();

        // Test reverse lookup
        assert_eq!(db.get_leaf_position(&mmr_id, hash1).unwrap(), Some(idx1));
        assert_eq!(db.get_leaf_position(&mmr_id, hash2).unwrap(), Some(idx2));
        assert_eq!(db.get_leaf_position(&mmr_id, [99u8; 32]).unwrap(), None);

        // Pop a leaf and verify index is removed
        db.pop_leaf(mmr_id.clone()).unwrap();
        assert_eq!(db.get_leaf_position(&mmr_id, hash2).unwrap(), None);
        assert_eq!(db.get_leaf_position(&mmr_id, hash1).unwrap(), Some(idx1));
    }

    #[test]
    fn test_multiple_mmr_instances() {
        let db = setup();

        let account1 = AccountId::zero();
        let account2 = AccountId::from([1u8; 32]);

        // Append to ASM MMR
        let asm_hash = [10u8; 32];
        db.append_leaf(MmrId::Asm, asm_hash).unwrap();

        // Append to account1 MMR
        let acc1_hash = [20u8; 32];
        db.append_leaf(MmrId::SnarkMsg(account1), acc1_hash)
            .unwrap();

        // Append to account2 MMR
        let acc2_hash = [30u8; 32];
        db.append_leaf(MmrId::SnarkMsg(account2), acc2_hash)
            .unwrap();

        // Verify each MMR is independent
        assert_eq!(db.num_leaves(MmrId::Asm).unwrap(), 1);
        assert_eq!(db.num_leaves(MmrId::SnarkMsg(account1)).unwrap(), 1);
        assert_eq!(db.num_leaves(MmrId::SnarkMsg(account2)).unwrap(), 1);

        // Verify hash index works per MMR
        assert_eq!(
            db.get_leaf_position(&MmrId::Asm, asm_hash).unwrap(),
            Some(0)
        );
        assert_eq!(
            db.get_leaf_position(&MmrId::SnarkMsg(account1), acc1_hash)
                .unwrap(),
            Some(0)
        );
        assert_eq!(
            db.get_leaf_position(&MmrId::SnarkMsg(account2), acc2_hash)
                .unwrap(),
            Some(0)
        );

        // Cross-MMR lookup should fail
        assert_eq!(db.get_leaf_position(&MmrId::Asm, acc1_hash).unwrap(), None);
        assert_eq!(
            db.get_leaf_position(&MmrId::SnarkMsg(account1), asm_hash)
                .unwrap(),
            None
        );
    }
}
