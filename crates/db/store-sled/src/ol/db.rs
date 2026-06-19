use sled::transaction::ConflictableTransactionError;
use strata_db_types::{
    DbError, DbResult,
    traits::{BlockStatus, OLBlockDatabase},
};
use strata_identifiers::{OLBlockCommitment, OLBlockId, Slot};
use strata_ol_chain_types_new::OLBlock;
use typed_sled::error::Error as TSledError;

use super::schemas::{
    OLBlockHeightSchema, OLBlockHighWatermarkSchema, OLBlockSchema, OLBlockStatusSchema,
    OLCanonicalBlockSchema,
};
use crate::{
    define_sled_database,
    utils::{first, to_db_error},
};

const OL_BLOCK_HIGH_WATERMARK_KEY: u8 = 0;

define_sled_database!(
    pub struct OLBlockDBSled {
        blk_tree: OLBlockSchema,
        blk_status_tree: OLBlockStatusSchema,
        blk_height_tree: OLBlockHeightSchema,
        blk_high_watermark_tree: OLBlockHighWatermarkSchema,
        blk_canonical_tree: OLCanonicalBlockSchema,
    }
);

impl OLBlockDatabase for OLBlockDBSled {
    fn put_block_data(&self, block: OLBlock) -> DbResult<()> {
        let slot = block.header().slot();
        let block_id = block.header().compute_blkid();

        self.config
            .with_retry(
                (&self.blk_tree, &self.blk_status_tree, &self.blk_height_tree),
                |(bt, bst, bht)| {
                    let mut blocks_at_slot = bht.get(&slot)?.unwrap_or(Vec::new());
                    let is_new = !blocks_at_slot.contains(&block_id);

                    if is_new {
                        blocks_at_slot.push(block_id);
                        bht.insert(&slot, &blocks_at_slot)?;

                        // Only set status to Unchecked for new blocks
                        // This preserves Valid/Invalid status if block is re-inserted
                        bst.insert(&block_id, &BlockStatus::Unchecked)?;
                    }

                    bt.insert(&block_id, &block)?;
                    Ok(())
                },
            )
            .map_err(to_db_error)?;
        Ok(())
    }

    fn get_block_high_watermark(&self) -> DbResult<Option<OLBlockCommitment>> {
        Ok(self
            .blk_high_watermark_tree
            .get(&OL_BLOCK_HIGH_WATERMARK_KEY)?)
    }

    fn put_block_data_with_high_watermark(&self, block: OLBlock) -> DbResult<OLBlockCommitment> {
        let slot = block.header().slot();
        let block_id = block.header().compute_blkid();
        let commitment = OLBlockCommitment::new(slot, block_id);

        self.config.with_retry(
            (
                &self.blk_tree,
                &self.blk_status_tree,
                &self.blk_height_tree,
                &self.blk_high_watermark_tree,
            ),
            |(bt, bst, bht, hwt)| {
                if let Some(current) = hwt.get(&OL_BLOCK_HIGH_WATERMARK_KEY)?
                    && commitment.slot() <= current.slot()
                {
                    return Err(ConflictableTransactionError::Abort(TSledError::abort(
                        DbError::BlockHighWatermarkConflict {
                            attempted: commitment,
                            current,
                        },
                    )));
                }

                let mut blocks_at_slot = bht.get(&slot)?.unwrap_or(Vec::new());
                let is_new = !blocks_at_slot.contains(&block_id);

                if is_new {
                    blocks_at_slot.push(block_id);
                    bht.insert(&slot, &blocks_at_slot)?;

                    // Only set status to Unchecked for new blocks.
                    // This preserves Valid/Invalid status if block is re-inserted.
                    bst.insert(&block_id, &BlockStatus::Unchecked)?;
                }

                bt.insert(&block_id, &block)?;
                hwt.insert(&OL_BLOCK_HIGH_WATERMARK_KEY, &commitment)?;

                Ok(commitment)
            },
        )
    }

    fn clear_block_high_watermark(&self, expected: OLBlockCommitment) -> DbResult<bool> {
        self.config
            .with_retry((&self.blk_high_watermark_tree,), |(hwt,)| {
                let Some(current) = hwt.get(&OL_BLOCK_HIGH_WATERMARK_KEY)? else {
                    return Ok(false);
                };

                if current != expected {
                    return Ok(false);
                }

                hwt.remove(&OL_BLOCK_HIGH_WATERMARK_KEY)?;
                Ok(true)
            })
            .map_err(to_db_error)
    }

    fn rollback_block_high_watermark(&self, target: OLBlockCommitment) -> DbResult<bool> {
        self.config.with_retry(
            (&self.blk_tree, &self.blk_high_watermark_tree),
            |(bt, hwt)| {
                let target_block_id = *target.blkid();
                let Some(target_block) = bt.get(&target_block_id)? else {
                    return Err(ConflictableTransactionError::Abort(TSledError::abort(
                        DbError::NonExistentEntry,
                    )));
                };

                if target_block.header().slot() != target.slot() {
                    return Err(ConflictableTransactionError::Abort(TSledError::abort(
                        DbError::InvalidArgument,
                    )));
                }

                let Some(current) = hwt.get(&OL_BLOCK_HIGH_WATERMARK_KEY)? else {
                    return Ok(false);
                };

                if current.slot() <= target.slot() {
                    return Ok(false);
                }

                hwt.insert(&OL_BLOCK_HIGH_WATERMARK_KEY, &target)?;
                Ok(true)
            },
        )
    }

    fn get_block_data(&self, id: OLBlockId) -> DbResult<Option<OLBlock>> {
        Ok(self.blk_tree.get(&id)?)
    }

    fn del_block_data(&self, id: OLBlockId) -> DbResult<bool> {
        // Need to find which slot this block is at
        let block = match self.get_block_data(id)? {
            Some(b) => b,
            None => return Ok(false),
        };
        let slot = block.header().slot();

        self.config
            .with_retry(
                (&self.blk_tree, &self.blk_status_tree, &self.blk_height_tree),
                |(bt, bst, bht)| {
                    let mut blocks_at_slot = bht.get(&slot)?.unwrap_or(Vec::new());
                    blocks_at_slot.retain(|&bid| bid != id);

                    bt.remove(&id)?;
                    bst.remove(&id)?;
                    bht.insert(&slot, &blocks_at_slot)?;
                    Ok(true)
                },
            )
            .map_err(to_db_error)
    }

    fn set_block_status(&self, id: OLBlockId, status: BlockStatus) -> DbResult<bool> {
        // Check if block exists before setting status
        if self.get_block_data(id)?.is_none() {
            return Err(DbError::NonExistentEntry);
        }
        self.blk_status_tree.insert(&id, &status)?;
        Ok(true)
    }

    fn get_blocks_at_height(&self, slot: u64) -> DbResult<Vec<OLBlockId>> {
        Ok(self.blk_height_tree.get(&slot)?.unwrap_or(Vec::new()))
    }

    fn get_block_status(&self, id: OLBlockId) -> DbResult<Option<BlockStatus>> {
        Ok(self.blk_status_tree.get(&id)?)
    }

    fn get_tip_slot(&self) -> DbResult<Slot> {
        let bht = &self.blk_height_tree;
        let mut slot = bht.last()?.map(first).ok_or(DbError::NotBootstrapped)?;

        loop {
            let blocks = self.get_blocks_at_height(slot)?;
            // Check if any valid blocks exist at this slot.
            // Multiple blocks at the same slot can be marked Valid during forks.
            let has_valid = blocks
                .into_iter()
                .filter_map(|blkid| match self.get_block_status(blkid) {
                    Ok(Some(BlockStatus::Valid)) => Some(Ok(())),
                    Ok(_) => None,
                    Err(e) => Some(Err(e)),
                })
                .collect::<Result<Vec<_>, _>>()?;

            // Return the highest slot that has at least one valid block.
            if !has_valid.is_empty() {
                return Ok(slot);
            }

            if slot == 0 {
                return Err(DbError::NotBootstrapped);
            }

            slot -= 1;
        }
    }

    fn get_canonical_block(&self, slot: Slot) -> DbResult<Option<OLBlockId>> {
        Ok(self.blk_canonical_tree.get(&slot)?)
    }

    fn replace_canonical_blocks_from(
        &self,
        start_slot: Slot,
        blocks: Vec<(Slot, OLBlockId)>,
    ) -> DbResult<()> {
        // First collect all slots to remove from the suffix.
        let mut slots_to_drop = Vec::new();
        for item in self.blk_canonical_tree.range(start_slot..)? {
            let (slot, _) = item?;
            slots_to_drop.push(slot);
        }

        // Now actually remove and insert new canonical blocks inside a transaction.
        self.config
            .with_retry((&self.blk_canonical_tree,), |(ct,)| {
                for slot in &slots_to_drop {
                    ct.remove(slot)?;
                }
                for (slot, id) in &blocks {
                    ct.insert(slot, id)?;
                }
                Ok(())
            })
            .map_err(to_db_error)
    }
}

#[cfg(feature = "test_utils")]
#[cfg(test)]
mod tests {
    use strata_db_tests::ol_block_db_tests;
    use strata_ol_chain_types_new::test_utils as ol_test_utils;

    use super::*;
    use crate::sled_db_test_setup;

    sled_db_test_setup!(OLBlockDBSled, ol_block_db_tests);
}
