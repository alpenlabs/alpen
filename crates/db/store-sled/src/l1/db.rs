use strata_asm_common::AsmManifest;
use strata_db_types::{DbResult, traits::*};
use strata_primitives::l1::L1BlockId;
use typed_sled::batch::SledBatch;

use super::schemas::{L1BlockSchema, L1BlocksByHeightSchema, L1CanonicalBlockSchema};
use crate::{
    define_sled_database,
    utils::{first, to_db_error},
};

define_sled_database!(
    pub struct L1DBSled {
        l1_blk_tree: L1BlockSchema,
        l1_canonical_tree: L1CanonicalBlockSchema,
        l1_blks_height_tree: L1BlocksByHeightSchema,
    }
);

impl L1DBSled {
    pub fn get_latest_block(&self) -> DbResult<Option<(u64, L1BlockId)>> {
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
                    blocks_at_height.push(*blockid);

                    bt.insert(blockid, &manifest)?;
                    bht.insert(&height, &blocks_at_height)?;

                    Ok(())
                },
            )
            .map_err(to_db_error)
    }

    fn set_canonical_chain_entry(&self, height: u64, blockid: L1BlockId) -> DbResult<()> {
        Ok(self.l1_canonical_tree.insert(&height, &blockid)?)
    }

    fn remove_canonical_chain_entries(&self, start_height: u64, end_height: u64) -> DbResult<()> {
        let mut batch = SledBatch::<L1CanonicalBlockSchema>::new();
        for height in (start_height..=end_height).rev() {
            batch.remove(height)?;
        }
        // Execute the batch
        self.l1_canonical_tree.apply_batch(batch)?;
        Ok(())
    }

    fn prune_to_height(&self, end_height: u64) -> DbResult<()> {
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

    fn get_canonical_chain_tip(&self) -> DbResult<Option<(u64, L1BlockId)>> {
        self.get_latest_block()
    }

    fn get_canonical_blockid_range(
        &self,
        start_idx: u64,
        end_idx: u64,
    ) -> DbResult<Vec<L1BlockId>> {
        let mut result = Vec::new();
        for height in start_idx..end_idx {
            if let Some(blockid) = self.l1_canonical_tree.get(&height)? {
                result.push(blockid);
            }
        }
        Ok(result)
    }

    fn get_canonical_blockid_at_height(&self, height: u64) -> DbResult<Option<L1BlockId>> {
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

    use super::*;
    use crate::sled_db_test_setup;

    sled_db_test_setup!(L1DBSled, l1_db_tests);
}
