use strata_asm_common::AsmManifest;
use strata_db_types::DbResult;
use strata_db_types::l1::L1Database;
use strata_primitives::L1Height;
use strata_primitives::l1::L1BlockId;
use typed_sled::batch::SledBatch;

use super::schemas::{L1BlockSchema, L1BlocksByHeightSchema, L1CanonicalBlockSchema};
use crate::define_sled_database;
use crate::utils::{first, to_db_error};

define_sled_database!(
    pub struct L1DBSled {
        l1_blk_tree: L1BlockSchema,
        l1_canonical_tree: L1CanonicalBlockSchema,
        l1_blks_height_tree: L1BlocksByHeightSchema,
    }
);

impl L1DBSled {
    pub fn get_latest_block(&self) -> DbResult<Option<(L1Height, L1BlockId)>> {
        Ok(self.l1_canonical_tree.last()?)
    }
}

impl L1Database for L1DBSled {
    fn put_block_data(&self, manifest: AsmManifest) -> DbResult<()> {
        let blockid = manifest.blkid();
        let height = manifest.height();

        self.config
            .with_retry(
                (&self.l1_blk_tree, &self.l1_blks_height_tree),
                |(bt, bht)| {
                    let mut blocks_at_height = bht.get(&height)?.unwrap_or_default();
                    if !blocks_at_height.contains(blockid) {
                        blocks_at_height.push(*blockid);
                    }

                    bt.insert(blockid, &manifest)?;
                    bht.insert(&height, &blocks_at_height)?;

                    Ok(())
                },
            )
            .map_err(to_db_error)
    }

    fn set_canonical_chain_entry(&self, height: L1Height, blockid: L1BlockId) -> DbResult<()> {
        Ok(self.l1_canonical_tree.insert(&height, &blockid)?)
    }

    fn remove_canonical_chain_entries(
        &self,
        start_height: L1Height,
        end_height: L1Height,
    ) -> DbResult<()> {
        let mut batch = SledBatch::<L1CanonicalBlockSchema>::new();
        for height in (start_height..=end_height).rev() {
            batch.remove(height)?;
        }
        // Execute the batch
        self.l1_canonical_tree.apply_batch(batch)?;
        Ok(())
    }

    fn prune_to_height(&self, end_height: L1Height) -> DbResult<()> {
        let earliest = self.l1_blks_height_tree.first()?.map(first);
        let Some(start_height) = earliest else {
            // empty db
            return Ok(());
        };

        self.config
            .with_retry(
                (
                    &self.l1_blk_tree,
                    &self.l1_blks_height_tree,
                    &self.l1_canonical_tree,
                ),
                |(bt, bht, ct)| {
                    for height in start_height..=end_height {
                        let blocks = bht.get(&height)?.unwrap_or_default();

                        bht.remove(&height)?;
                        ct.remove(&height)?;
                        for blockid in blocks {
                            bt.remove(&blockid)?;
                        }
                    }

                    Ok(())
                },
            )
            .map_err(to_db_error)?;
        Ok(())
    }

    fn get_canonical_chain_tip(&self) -> DbResult<Option<(L1Height, L1BlockId)>> {
        self.get_latest_block()
    }

    fn get_canonical_blockid_range(
        &self,
        start_idx: L1Height,
        end_idx: L1Height,
    ) -> DbResult<Vec<L1BlockId>> {
        let mut result = Vec::new();
        for height in start_idx..end_idx {
            if let Some(blockid) = self.l1_canonical_tree.get(&height)? {
                result.push(blockid);
            }
        }
        Ok(result)
    }

    fn get_canonical_blockid_at_height(&self, height: L1Height) -> DbResult<Option<L1BlockId>> {
        Ok(self.l1_canonical_tree.get(&height)?)
    }

    fn get_block_manifest(&self, blockid: L1BlockId) -> DbResult<Option<AsmManifest>> {
        Ok(self.l1_blk_tree.get(&blockid)?)
    }
}

#[cfg(feature = "test_utils")]
#[cfg(test)]
mod tests {
    use strata_db_tests::l1_db_tests;
    use strata_primitives::Buf32;

    use super::*;
    use crate::sled_db_test_setup;

    sled_db_test_setup!(L1DBSled, l1_db_tests);

    #[test]
    fn repeated_manifest_writes_preserve_unique_height_entries_and_forks() {
        let db = setup_db();
        let height = 100;
        let block_id = L1BlockId::from(Buf32::from([1; 32]));
        let fork_id = L1BlockId::from(Buf32::from([2; 32]));
        let root = Buf32::from([3; 32]).into();
        let manifest = AsmManifest::new(height, block_id, root, vec![]).unwrap();
        let fork = AsmManifest::new(height, fork_id, root, vec![]).unwrap();

        db.put_block_data(manifest.clone()).unwrap();
        db.put_block_data(manifest).unwrap();
        assert_eq!(
            db.l1_blks_height_tree.get(&height).unwrap(),
            Some(vec![block_id])
        );

        db.put_block_data(fork.clone()).unwrap();
        let updated =
            AsmManifest::new(height, block_id, Buf32::from([4; 32]).into(), vec![]).unwrap();
        db.put_block_data(updated.clone()).unwrap();
        db.put_block_data(fork.clone()).unwrap();
        assert_eq!(
            db.l1_blks_height_tree.get(&height).unwrap(),
            Some(vec![block_id, fork_id])
        );
        assert_eq!(db.get_block_manifest(block_id).unwrap(), Some(updated));
        assert_eq!(db.get_block_manifest(fork_id).unwrap(), Some(fork));
    }
}
