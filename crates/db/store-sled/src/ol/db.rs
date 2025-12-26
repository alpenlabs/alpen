use strata_db_types::{
    DbError, DbResult,
    traits::{BlockStatus, OLBlockDatabase},
};
use strata_identifiers::{OLBlockId, Slot};
use strata_ol_chain_types_new::OLBlock;

use super::schemas::{OLBlockHeightSchema, OLBlockSchema, OLBlockStatusSchema};
use crate::{
    define_sled_database,
    utils::{first, to_db_error},
};

define_sled_database!(
    pub struct OLBlockDBSled {
        blk_tree: OLBlockSchema,
        blk_status_tree: OLBlockStatusSchema,
        blk_height_tree: OLBlockHeightSchema,
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
