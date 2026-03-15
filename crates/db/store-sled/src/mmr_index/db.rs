use strata_db_types::{
    DbError, DbResult, LeafPos, MmrBatchWrite, MmrIndexPrecondition, MmrNodePos, MmrNodeTable,
    NodePos, RawMmrId, traits::MmrIndexDatabase,
};
use strata_identifiers::Hash;
use typed_sled::{error, tree::SledTransactionalTree};

use super::schemas::{MmrIndexLeafCountSchema, MmrIndexNodeSchema, MmrIndexPreimageSchema};
use crate::define_sled_database;

fn make_precond_fail_abort(
    mmr_id: RawMmrId,
    detail: String,
) -> error::ConflictableTransactionError<error::Error> {
    error::ConflictableTransactionError::Abort(error::Error::abort(
        DbError::MmrPreconditionFailed { mmr_id, detail },
    ))
}

define_sled_database!(
    pub struct MmrIndexDb {
        node_tree: MmrIndexNodeSchema,
        preimage_tree: MmrIndexPreimageSchema,
        leaf_count_tree: MmrIndexLeafCountSchema,
    }
);

impl MmrIndexDatabase for MmrIndexDb {
    /// Returns a single node hash by `(mmr_id, NodePos)`.
    fn get_node(&self, mmr_id: RawMmrId, pos: NodePos) -> DbResult<Option<Hash>> {
        Ok(self.node_tree.get(&(mmr_id, pos))?)
    }

    /// Returns an optional leaf preimage by `(mmr_id, LeafPos)`.
    fn get_preimage(&self, mmr_id: RawMmrId, pos: LeafPos) -> DbResult<Option<Vec<u8>>> {
        Ok(self.preimage_tree.get(&(mmr_id, pos))?)
    }

    /// Returns current leaf count for a namespace.
    ///
    /// Absent entries are treated as zero leaves.
    fn get_leaf_count(&self, mmr_id: RawMmrId) -> DbResult<u64> {
        Ok(self.leaf_count_tree.get(&mmr_id)?.unwrap_or(0))
    }

    /// Fetches requested nodes and available ancestors from the node tree.
    ///
    /// For each requested node, this walks upward through parents and includes
    /// every present node until the first missing node in that chain.
    /// When `preimages` is true, leaf preimages are included for requested
    /// height-0 nodes if present.
    ///
    /// All reads are executed in one sled transaction so the returned table is
    /// assembled from a single consistent snapshot.
    fn fetch_node_paths(&self, nodes: Vec<MmrNodePos>, preimages: bool) -> DbResult<MmrNodeTable> {
        self.config.with_retry(
            (&self.node_tree, &self.preimage_tree),
            |(nt, pt): (
                SledTransactionalTree<MmrIndexNodeSchema>,
                SledTransactionalTree<MmrIndexPreimageSchema>,
            )| {
                let mut out = MmrNodeTable::default();

                for node_ref in &nodes {
                    let mmr_id = node_ref.id().clone();
                    let requested_pos = node_ref.pos();
                    let mut current = Some(requested_pos);

                    // Walk upward from the requested node and stop at the first absent parent.
                    while let Some(pos) = current {
                        let Some(hash) = nt.get(&(mmr_id.clone(), pos))? else {
                            break;
                        };
                        out.get_or_create_table_mut(mmr_id.clone())
                            .put_node(pos, hash);
                        current = pos.parent();
                    }

                    // Preimages are leaf-only and optional for this fetch.
                    if preimages && requested_pos.height() == 0 {
                        let leaf_pos = LeafPos::new(requested_pos.index());
                        if let Some(preimage) = pt.get(&(mmr_id.clone(), leaf_pos))? {
                            out.get_or_create_table_mut(mmr_id.clone())
                                .put_preimage(leaf_pos, preimage);
                        }
                    }
                }

                Ok(out)
            },
        )
    }

    /// Applies a multi-MMR atomic update with compare-and-set preconditions.
    ///
    /// Preconditions are validated first for every per-MMR batch; if any check
    /// fails, the transaction aborts and no write is applied.
    fn apply_update(&self, batch: MmrBatchWrite) -> DbResult<()> {
        self.config.with_retry(
            (&self.node_tree, &self.preimage_tree, &self.leaf_count_tree),
            |(nt, pt, lc): (
                SledTransactionalTree<MmrIndexNodeSchema>,
                SledTransactionalTree<MmrIndexPreimageSchema>,
                SledTransactionalTree<MmrIndexLeafCountSchema>,
            )| {
                // Pass 1: validate all preconditions before any write is applied.
                for (mmr_id, mmr_batch) in batch.batches() {
                    if let Some(expected_leaf_count) = mmr_batch.expected_leaf_count() {
                        let current_leaf_count = lc.get(mmr_id)?.unwrap_or(0);
                        if current_leaf_count != expected_leaf_count {
                            return Err(make_precond_fail_abort(
                                mmr_id.clone(),
                                format!(
                                    "leaf_count: expected {expected_leaf_count}, got {current_leaf_count}"
                                ),
                            ));
                        }
                    }

                    for precondition in mmr_batch.preconditions() {
                        match precondition {
                            MmrIndexPrecondition::Node { pos, expected } => {
                                let current = nt.get(&(mmr_id.clone(), *pos))?;
                                if current != *expected {
                                    return Err(make_precond_fail_abort(
                                        mmr_id.clone(),
                                        format!(
                                            "node at {pos:?}: expected {expected:?}, got {current:?}"
                                        ),
                                    ));
                                }
                            }
                            MmrIndexPrecondition::Preimage { pos, expected } => {
                                let current = pt.get(&(mmr_id.clone(), *pos))?;
                                if current != *expected {
                                    return Err(make_precond_fail_abort(
                                        mmr_id.clone(),
                                        format!(
                                            "preimage at {pos:?}: expected {expected:?}, got {current:?}"
                                        ),
                                    ));
                                }
                            }
                        }
                    }
                }

                // Pass 2: apply writes only if all preconditions succeeded.
                for (mmr_id, mmr_batch) in batch.batches() {
                    for (pos, hash) in mmr_batch.node_puts() {
                        nt.insert(&(mmr_id.clone(), pos), &hash)?;
                    }
                    for pos in mmr_batch.node_dels() {
                        nt.remove(&(mmr_id.clone(), pos))?;
                    }
                    for (pos, preimage) in mmr_batch.preimage_puts() {
                        pt.insert(&(mmr_id.clone(), pos), preimage)?;
                    }
                    for pos in mmr_batch.preimage_dels() {
                        pt.remove(&(mmr_id.clone(), pos))?;
                    }
                    if let Some(next_leaf_count) = mmr_batch.leaf_count() {
                        lc.insert(mmr_id, &next_leaf_count)?;
                    }
                }

                Ok(())
            },
        )
    }
}

#[cfg(feature = "test_utils")]
#[cfg(test)]
mod tests {
    use strata_db_tests::mmr_index_db_tests;
    use strata_db_types::{LeafPos, MmrBatchWrite, MmrNodePos, NodePos};
    use strata_identifiers::Hash;

    use super::*;
    use crate::sled_db_test_setup;

    sled_db_test_setup!(MmrIndexDb, mmr_index_db_tests);

    #[test]
    fn fetch_node_paths_returns_available_parent_chain() {
        let db = setup_db();
        let mmr_id = vec![0x11; 32];
        let leaf = NodePos::new(0, 3);
        let parent = NodePos::new(1, 1);
        let grandparent = NodePos::new(2, 0);

        let mut batch = MmrBatchWrite::default();
        {
            let mmr_batch = batch.entry(mmr_id.clone());
            mmr_batch.put_node(leaf, Hash::from([1u8; 32]));
            mmr_batch.put_node(parent, Hash::from([2u8; 32]));
            mmr_batch.put_node(grandparent, Hash::from([3u8; 32]));
        }
        db.apply_update(batch).expect("seed nodes");

        let table = db
            .fetch_node_paths(vec![MmrNodePos::new(mmr_id.clone(), leaf)], false)
            .expect("fetch path");
        let per_mmr = table.get_table(&mmr_id).expect("mmr table");

        assert_eq!(per_mmr.get_node(leaf), Some(&Hash::from([1u8; 32])));
        assert_eq!(per_mmr.get_node(parent), Some(&Hash::from([2u8; 32])));
        assert_eq!(per_mmr.get_node(grandparent), Some(&Hash::from([3u8; 32])));
    }

    #[test]
    fn fetch_node_paths_preimages_toggle() {
        let db = setup_db();
        let mmr_id = vec![0x22; 32];
        let leaf = LeafPos::new(7);
        let preimage = vec![9u8, 8u8, 7u8];

        let mut batch = MmrBatchWrite::default();
        {
            let mmr_batch = batch.entry(mmr_id.clone());
            mmr_batch.put_node(leaf.node_pos(), Hash::from([4u8; 32]));
            mmr_batch.put_preimage(leaf, preimage.clone());
        }
        db.apply_update(batch).expect("seed node+preimage");

        let without_preimages = db
            .fetch_node_paths(
                vec![MmrNodePos::new(mmr_id.clone(), leaf.node_pos())],
                false,
            )
            .expect("fetch without preimages");
        let with_preimages = db
            .fetch_node_paths(vec![MmrNodePos::new(mmr_id.clone(), leaf.node_pos())], true)
            .expect("fetch with preimages");

        let table_without = without_preimages.get_table(&mmr_id).expect("table without");
        let table_with = with_preimages.get_table(&mmr_id).expect("table with");

        assert_eq!(table_without.get_preimage(leaf), None);
        assert_eq!(table_with.get_preimage(leaf), Some(&preimage));
    }

    #[test]
    fn get_leaf_count_defaults_to_zero_and_persists_updates() {
        let db = setup_db();
        let mmr_id = vec![0x33; 32];

        assert_eq!(db.get_leaf_count(mmr_id.clone()).expect("initial count"), 0);

        let mut batch = MmrBatchWrite::default();
        batch.entry(mmr_id.clone()).set_leaf_count(7);
        db.apply_update(batch).expect("set leaf count");

        assert_eq!(db.get_leaf_count(mmr_id).expect("updated count"), 7);
    }

    #[test]
    fn apply_update_rejects_stale_expected_leaf_count_and_rolls_back() {
        let db = setup_db();
        let mmr_id = vec![0x44; 32];
        let leaf = NodePos::new(0, 0);
        let hash = Hash::from([0x55; 32]);

        let mut setup = MmrBatchWrite::default();
        setup.entry(mmr_id.clone()).set_leaf_count(2);
        db.apply_update(setup).expect("seed count");

        let mut stale = MmrBatchWrite::default();
        {
            let mmr_batch = stale.entry(mmr_id.clone());
            mmr_batch.set_expected_leaf_count(1);
            mmr_batch.put_node(leaf, hash);
            mmr_batch.set_leaf_count(3);
        }

        assert!(db.apply_update(stale).is_err());
        assert_eq!(
            db.get_leaf_count(mmr_id.clone()).expect("count after fail"),
            2
        );
        assert_eq!(db.get_node(mmr_id, leaf).expect("node after fail"), None);
    }
}
