use ssz_types::FixedBytes;
use strata_db_types::{
    DbError, DbResult,
    mmr_helpers::{BitManipulatedMmrAlgorithm, MmrAlgorithm, MmrMetadata},
    traits::GlobalMmrDatabase,
};
use strata_identifiers::{Hash, RawMmrId};
use strata_merkle::CompactMmr64B32 as CompactMmr64;
use strata_primitives::buf::Buf32;
use typed_sled::{error, tree::SledTransactionalTree};

use super::schemas::{
    GlobalMmrHashPositionSchema, GlobalMmrMetaSchema, GlobalMmrNodeSchema, GlobalMmrPreimageSchema,
};
use crate::define_sled_database;

define_sled_database!(
    pub struct GlobalMmrDb {
        node_tree: GlobalMmrNodeSchema,
        meta_tree: GlobalMmrMetaSchema,
        hash_pos_tree: GlobalMmrHashPositionSchema,
        preimage_tree: GlobalMmrPreimageSchema,
    }
);

impl GlobalMmrDb {
    fn ensure_mmr_metadata(&self, mmr_id: &[u8]) -> DbResult<()> {
        let key = mmr_id.to_vec();
        if self.meta_tree.get(&key)?.is_none() {
            let metadata = MmrMetadata::empty();
            self.meta_tree.insert(&key, &metadata)?;
        }
        Ok(())
    }

    fn load_mmr_metadata(&self, mmr_id: &[u8]) -> DbResult<MmrMetadata> {
        self.meta_tree.get(&mmr_id.to_vec())?.ok_or_else(|| {
            DbError::Other(format!("MMR metadata not found for mmr_id {:?}", mmr_id))
        })
    }

    fn get_mmr_node(&self, mmr_id: &[u8], pos: u64) -> DbResult<Option<Hash>> {
        Ok(self.node_tree.get(&(mmr_id.to_vec(), pos))?)
    }

    /// Get the position of a leaf by its hash (reverse lookup)
    pub fn get_leaf_position(&self, mmr_id: &[u8], hash: Hash) -> DbResult<Option<u64>> {
        Ok(self.hash_pos_tree.get(&(mmr_id.to_vec(), hash))?)
    }

    fn append_leaf_in_transaction<A: MmrAlgorithm>(
        mmr_id: RawMmrId,
        hash: Hash,
        nt: SledTransactionalTree<GlobalMmrNodeSchema>,
        mt: SledTransactionalTree<GlobalMmrMetaSchema>,
        hpt: SledTransactionalTree<GlobalMmrHashPositionSchema>,
    ) -> error::Result<u64> {
        let metadata = mt
            .get(&mmr_id)?
            .expect("MMR metadata must exist after ensure_mmr_metadata");

        let result = A::append_leaf(hash.0, &metadata, |pos| {
            nt.get(&(mmr_id.clone(), pos))?
                .map(|buf| buf.0)
                .ok_or_else(|| DbError::Other(format!("MMR node not found at pos {}", pos)))
        })
        .map_err(error::Error::abort)?;

        for (pos, node_hash) in &result.nodes_to_write {
            nt.insert(&(mmr_id.clone(), *pos), &Buf32(*node_hash))?;
            hpt.insert(&(mmr_id.clone(), Buf32(*node_hash)), pos)?;
        }

        hpt.insert(&(mmr_id.clone(), hash), &result.leaf_index)?;
        mt.insert(&mmr_id, &result.new_metadata)?;

        Ok(result.leaf_index)
    }
}

impl GlobalMmrDatabase for GlobalMmrDb {
    type MmrAlgorithm = BitManipulatedMmrAlgorithm;

    fn append_leaf(&self, mmr_id: RawMmrId, hash: Hash) -> DbResult<u64> {
        self.ensure_mmr_metadata(&mmr_id)?;

        self.config.with_retry(
            (&self.node_tree, &self.meta_tree, &self.hash_pos_tree),
            |(nt, mt, hpt)| {
                Ok(Self::append_leaf_in_transaction::<Self::MmrAlgorithm>(
                    mmr_id.clone(),
                    hash,
                    nt,
                    mt,
                    hpt,
                )?)
            },
        )
    }

    fn get_node(&self, mmr_id: RawMmrId, pos: u64) -> DbResult<Option<Hash>> {
        self.get_mmr_node(&mmr_id, pos)
    }

    fn get_mmr_size(&self, mmr_id: RawMmrId) -> DbResult<u64> {
        self.ensure_mmr_metadata(&mmr_id)?;
        let metadata = self.load_mmr_metadata(&mmr_id)?;
        Ok(metadata.mmr_size)
    }

    fn get_num_leaves(&self, mmr_id: RawMmrId) -> DbResult<u64> {
        self.ensure_mmr_metadata(&mmr_id)?;
        let metadata = self.load_mmr_metadata(&mmr_id)?;
        Ok(metadata.num_leaves)
    }

    fn get_peaks(&self, mmr_id: RawMmrId) -> DbResult<Vec<Hash>> {
        self.load_mmr_metadata(&mmr_id)
            .map(|m| m.peaks.into_iter().collect())
    }

    fn get_compact(&self, mmr_id: RawMmrId) -> DbResult<CompactMmr64> {
        let metadata = self
            .load_mmr_metadata(&mmr_id)
            .unwrap_or_else(|_| MmrMetadata::empty());

        let roots_vec: Vec<_> = metadata
            .peaks
            .iter()
            .map(|buf| FixedBytes::<32>::from(buf.0))
            .collect();

        Ok(CompactMmr64 {
            entries: metadata.num_leaves,
            cap_log2: 64,
            roots: roots_vec.into(),
        })
    }

    fn pop_leaf(&self, mmr_id: RawMmrId) -> DbResult<Option<Hash>> {
        self.ensure_mmr_metadata(&mmr_id)?;

        self.config.with_retry(
            (&self.node_tree, &self.meta_tree, &self.hash_pos_tree),
            |(nt, mt, hpt)| {
                let metadata = mt
                    .get(&mmr_id)?
                    .expect("MMR metadata must exist after ensure_mmr_metadata");

                let result = Self::MmrAlgorithm::pop_leaf(&metadata, |pos| {
                    nt.get(&(mmr_id.clone(), pos))?
                        .map(|x| x.0)
                        .ok_or_else(|| DbError::Other(format!("MMR node not found at pos {}", pos)))
                })
                .map_err(error::Error::abort)?;

                let Some(result) = result else {
                    return Ok(None);
                };

                for pos in &result.nodes_to_remove {
                    nt.remove(&(mmr_id.clone(), *pos))?;
                }

                mt.insert(&mmr_id, &result.new_metadata)?;

                // Remove hash -> position mapping for the popped leaf
                hpt.remove(&(mmr_id.clone(), Buf32(result.leaf_hash)))?;

                Ok(Some(result.leaf_hash.into()))
            },
        )
    }

    fn append_leaf_with_preimage(
        &self,
        mmr_id: RawMmrId,
        hash: Hash,
        preimage: Vec<u8>,
    ) -> DbResult<u64> {
        self.ensure_mmr_metadata(&mmr_id)?;

        self.config.with_retry(
            (
                &self.node_tree,
                &self.meta_tree,
                &self.hash_pos_tree,
                &self.preimage_tree,
            ),
            |(nt, mt, hpt, pit)| {
                let leaf_index = Self::append_leaf_in_transaction::<Self::MmrAlgorithm>(
                    mmr_id.clone(),
                    hash,
                    nt,
                    mt,
                    hpt,
                )?;
                pit.insert(&(mmr_id.clone(), leaf_index), &preimage)?;
                Ok(leaf_index)
            },
        )
    }

    fn get_preimage(&self, mmr_id: RawMmrId, index: u64) -> DbResult<Option<Vec<u8>>> {
        Ok(self.preimage_tree.get(&(mmr_id, index))?)
    }
}

#[cfg(feature = "test_utils")]
#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::test_utils::{get_test_sled_config, get_test_sled_db};

    fn setup() -> GlobalMmrDb {
        let db = Arc::new(get_test_sled_db());
        let config = get_test_sled_config();
        GlobalMmrDb::new(db, config).unwrap()
    }

    #[test]
    fn test_append_single_leaf() {
        let db = setup();
        let mmr_id = vec![1u8]; // Simple byte identifier
        let hash = [1u8; 32].into();

        // Initially empty
        assert_eq!(db.get_num_leaves(mmr_id.clone()).unwrap(), 0);
        assert_eq!(db.get_mmr_size(mmr_id.clone()).unwrap(), 0);

        // Append first leaf
        let idx = db.append_leaf(mmr_id.clone(), hash).unwrap();
        assert_eq!(idx, 0);
        assert_eq!(db.get_num_leaves(mmr_id.clone()).unwrap(), 1);
        assert_eq!(db.get_mmr_size(mmr_id.clone()).unwrap(), 1);

        // Can retrieve the node
        let node = db.get_node(mmr_id, 0).unwrap().unwrap();
        assert_eq!(node, hash);
    }

    #[test]
    fn test_append_multiple_leaves() {
        let db = setup();
        let mmr_id = vec![1u8]; // ASM MMR identifier

        // Append 7 leaves to create a complete tree
        let hashes: Vec<Hash> = (0..7).map(|i| [i; 32].into()).collect();

        for (expected_idx, hash) in hashes.iter().enumerate() {
            let idx = db.append_leaf(mmr_id.clone(), *hash).unwrap();
            assert_eq!(idx as usize, expected_idx);
        }

        assert_eq!(db.get_num_leaves(mmr_id.clone()).unwrap(), 7);
        // 7 leaves with 3 peaks -> mmr_size = 2*7 - 3 = 11
        assert_eq!(db.get_mmr_size(mmr_id.clone()).unwrap(), 11);
    }

    #[test]
    fn test_get_node_positions() {
        let db = setup();
        let mmr_id = vec![1u8]; // ASM MMR identifier

        // Append 4 leaves
        let hashes: Vec<Hash> = (0..4).map(|i| [i; 32].into()).collect();

        for hash in &hashes {
            db.append_leaf(mmr_id.clone(), *hash).unwrap();
        }

        // MMR with 4 leaves has 7 nodes:
        // Position: [0, 1, 2, 3, 4, 5, 6]
        // Height:   [0, 0, 1, 0, 0, 1, 2]
        // Leaves:   [0, 1, x, 2, 3, x, x]

        // Verify leaf positions
        assert_eq!(
            db.get_node(mmr_id.clone(), 0).unwrap().unwrap(),
            [0u8; 32].into()
        );
        assert_eq!(
            db.get_node(mmr_id.clone(), 1).unwrap().unwrap(),
            [1u8; 32].into()
        );
        assert_eq!(
            db.get_node(mmr_id.clone(), 3).unwrap().unwrap(),
            [2u8; 32].into()
        );
        assert_eq!(
            db.get_node(mmr_id.clone(), 4).unwrap().unwrap(),
            [3u8; 32].into()
        );

        // Internal nodes exist
        assert!(db.get_node(mmr_id.clone(), 2).is_ok());
        assert!(db.get_node(mmr_id.clone(), 5).is_ok());
        assert!(db.get_node(mmr_id.clone(), 6).is_ok());
    }

    #[test]
    fn test_peak_roots() {
        let db = setup();
        let mmr_id = vec![1u8]; // ASM MMR identifier

        // Single leaf: one peak
        db.append_leaf(mmr_id.clone(), [1u8; 32].into()).unwrap();
        let peaks = db.get_peaks(mmr_id.clone()).unwrap();
        assert_eq!(peaks.len(), 1);

        // Two leaves: one peak (merged)
        db.append_leaf(mmr_id.clone(), [2u8; 32].into()).unwrap();
        let peaks = db.get_peaks(mmr_id.clone()).unwrap();
        assert_eq!(peaks.len(), 1);

        // Three leaves: two peaks
        db.append_leaf(mmr_id.clone(), [3u8; 32].into()).unwrap();
        let peaks = db.get_peaks(mmr_id.clone()).unwrap();
        assert_eq!(peaks.len(), 2);

        // Four leaves: one peak (complete tree)
        db.append_leaf(mmr_id.clone(), [4u8; 32].into()).unwrap();
        let peaks = db.get_peaks(mmr_id.clone()).unwrap();
        assert_eq!(peaks.len(), 1);
    }

    #[test]
    fn test_pop_leaf_empty() {
        let db = setup();
        let mmr_id = vec![1u8]; // ASM MMR identifier

        // Popping from empty MMR returns None
        let result = db.pop_leaf(mmr_id.clone()).unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn test_pop_leaf_single() {
        let db = setup();
        let mmr_id = vec![1u8]; // ASM MMR identifier

        let hash = [42u8; 32].into();
        db.append_leaf(mmr_id.clone(), hash).unwrap();

        assert_eq!(db.get_num_leaves(mmr_id.clone()).unwrap(), 1);

        let popped = db.pop_leaf(mmr_id.clone()).unwrap();
        assert_eq!(popped, Some(hash));
        assert_eq!(db.get_num_leaves(mmr_id.clone()).unwrap(), 0);
        assert_eq!(db.get_mmr_size(mmr_id.clone()).unwrap(), 0);
    }

    #[test]
    fn test_pop_leaf_multiple() {
        let db = setup();
        let mmr_id = vec![1u8]; // ASM MMR identifier

        // Append several leaves
        let hashes: Vec<Hash> = (0..5).map(|i| [i; 32].into()).collect();
        for hash in &hashes {
            db.append_leaf(mmr_id.clone(), *hash).unwrap();
        }

        assert_eq!(db.get_num_leaves(mmr_id.clone()).unwrap(), 5);

        // Pop last leaf
        let popped = db.pop_leaf(mmr_id.clone()).unwrap();
        assert_eq!(popped, Some([4u8; 32].into()));
        assert_eq!(db.get_num_leaves(mmr_id.clone()).unwrap(), 4);

        // Pop another
        let popped = db.pop_leaf(mmr_id.clone()).unwrap();
        assert_eq!(popped, Some([3u8; 32].into()));
        assert_eq!(db.get_num_leaves(mmr_id.clone()).unwrap(), 3);
    }

    #[test]
    fn test_append_after_pop() {
        let db = setup();
        let mmr_id = vec![1u8]; // ASM MMR identifier

        // Append 3 leaves
        db.append_leaf(mmr_id.clone(), [1u8; 32].into()).unwrap();
        db.append_leaf(mmr_id.clone(), [2u8; 32].into()).unwrap();
        db.append_leaf(mmr_id.clone(), [3u8; 32].into()).unwrap();

        // Pop one
        db.pop_leaf(mmr_id.clone()).unwrap();
        assert_eq!(db.get_num_leaves(mmr_id.clone()).unwrap(), 2);

        // Append again
        let idx = db.append_leaf(mmr_id.clone(), [4u8; 32].into()).unwrap();
        assert_eq!(idx, 2);
        assert_eq!(db.get_num_leaves(mmr_id.clone()).unwrap(), 3);
    }

    #[test]
    fn test_to_compact() {
        let db = setup();
        let mmr_id = vec![1u8]; // ASM MMR identifier

        // Empty MMR
        let compact = db.get_compact(mmr_id.clone()).unwrap();
        assert_eq!(compact.entries, 0);

        // Add some leaves
        for i in 0..4 {
            db.append_leaf(mmr_id.clone(), [i; 32].into()).unwrap();
        }

        let compact = db.get_compact(mmr_id.clone()).unwrap();
        assert_eq!(compact.entries, 4);
        assert_eq!(compact.cap_log2, 64);
        assert!(!compact.roots.is_empty());
    }

    #[test]
    fn test_mmr_size_formula() {
        let db = setup();
        let mmr_id = vec![1u8]; // ASM MMR identifier

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
            while db.get_num_leaves(mmr_id.clone()).unwrap() > 0 {
                db.pop_leaf(mmr_id.clone()).unwrap();
            }

            for i in 0..num_leaves {
                db.append_leaf(mmr_id.clone(), [i; 32].into()).unwrap();
            }

            assert_eq!(
                db.get_mmr_size(mmr_id.clone()).unwrap(),
                expected_size,
                "MMR size mismatch for {} leaves",
                num_leaves
            );
        }
    }

    #[test]
    fn test_hash_index_lookup() {
        let db = setup();
        let mmr_id = vec![1u8]; // ASM MMR identifier

        let hash1 = [1u8; 32].into();
        let hash2 = [2u8; 32].into();

        // Append leaves
        let idx1 = db.append_leaf(mmr_id.clone(), hash1).unwrap();
        let idx2 = db.append_leaf(mmr_id.clone(), hash2).unwrap();

        // Test reverse lookup
        assert_eq!(db.get_leaf_position(&mmr_id, hash1).unwrap(), Some(idx1));
        assert_eq!(db.get_leaf_position(&mmr_id, hash2).unwrap(), Some(idx2));
        assert_eq!(
            db.get_leaf_position(&mmr_id, [99u8; 32].into()).unwrap(),
            None
        );

        // Pop a leaf and verify index is removed
        db.pop_leaf(mmr_id.clone()).unwrap();
        assert_eq!(db.get_leaf_position(&mmr_id, hash2).unwrap(), None);
        assert_eq!(db.get_leaf_position(&mmr_id, hash1).unwrap(), Some(idx1));
    }

    #[test]
    fn test_multiple_mmr_instances() {
        let db = setup();

        // Different MMR identifiers using raw bytes
        let asm_mmr_id = vec![1u8];
        let account1_mmr_id = vec![2u8, 0u8];
        let account2_mmr_id = vec![2u8, 1u8];

        // Append to ASM MMR
        let asm_hash = [10u8; 32].into();
        db.append_leaf(asm_mmr_id.clone(), asm_hash).unwrap();

        // Append to account1 MMR
        let acc1_hash = [20u8; 32].into();
        db.append_leaf(account1_mmr_id.clone(), acc1_hash).unwrap();

        // Append to account2 MMR
        let acc2_hash = [30u8; 32].into();
        db.append_leaf(account2_mmr_id.clone(), acc2_hash).unwrap();

        // Verify each MMR is independent
        assert_eq!(db.get_num_leaves(asm_mmr_id.clone()).unwrap(), 1);
        assert_eq!(db.get_num_leaves(account1_mmr_id.clone()).unwrap(), 1);
        assert_eq!(db.get_num_leaves(account2_mmr_id.clone()).unwrap(), 1);

        // Verify hash index works per MMR
        assert_eq!(
            db.get_leaf_position(&asm_mmr_id, asm_hash).unwrap(),
            Some(0)
        );
        assert_eq!(
            db.get_leaf_position(&account1_mmr_id, acc1_hash).unwrap(),
            Some(0)
        );
        assert_eq!(
            db.get_leaf_position(&account2_mmr_id, acc2_hash).unwrap(),
            Some(0)
        );

        // Cross-MMR lookup should fail
        assert_eq!(db.get_leaf_position(&asm_mmr_id, acc1_hash).unwrap(), None);
        assert_eq!(
            db.get_leaf_position(&account1_mmr_id, asm_hash).unwrap(),
            None
        );
    }
}
