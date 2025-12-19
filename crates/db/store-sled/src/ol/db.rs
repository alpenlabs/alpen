use strata_db_types::{
    DbError, DbResult,
    traits::{BlockStatus, OLBlockDatabase},
};
use strata_identifiers::{OLBlockCommitment, OLBlockId};
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
    fn put_block_data(&self, commitment: OLBlockCommitment, block: OLBlock) -> DbResult<()> {
        let block_id = *commitment.blkid();
        let slot = commitment.slot();

        self.config
            .with_retry(
                (&self.blk_tree, &self.blk_status_tree, &self.blk_height_tree),
                |(bt, bst, bht)| {
                    let mut block_height_data = bht.get(&slot)?.unwrap_or(Vec::new());
                    if !block_height_data.contains(&block_id) {
                        block_height_data.push(block_id);
                    }

                    bt.insert(&commitment, &block)?;
                    bst.insert(&block_id, &BlockStatus::Unchecked)?;
                    bht.insert(&slot, &block_height_data)?;
                    Ok(())
                },
            )
            .map_err(to_db_error)?;
        Ok(())
    }

    fn get_block_data(&self, commitment: OLBlockCommitment) -> DbResult<Option<OLBlock>> {
        Ok(self.blk_tree.get(&commitment)?)
    }

    fn del_block_data(&self, commitment: OLBlockCommitment) -> DbResult<()> {
        let block_id = *commitment.blkid();
        let slot = commitment.slot();

        self.config
            .with_retry(
                (&self.blk_tree, &self.blk_status_tree, &self.blk_height_tree),
                |(bt, bst, bht)| {
                    let mut block_height_data = bht.get(&slot)?.unwrap_or(Vec::new());
                    block_height_data.retain(|&bid| bid != block_id);

                    bt.remove(&commitment)?;
                    bst.remove(&block_id)?;
                    bht.insert(&slot, &block_height_data)?;
                    Ok(())
                },
            )
            .map_err(to_db_error)?;
        Ok(())
    }

    fn set_block_status(&self, id: OLBlockId, status: BlockStatus) -> DbResult<()> {
        // Check if block exists by trying to find it via commitment
        // Since we don't have a reverse mapping, we'll just set status if we can
        Ok(self.blk_status_tree.insert(&id, &status)?)
    }

    fn get_blocks_at_height(&self, slot: u64) -> DbResult<Vec<OLBlockId>> {
        Ok(self.blk_height_tree.get(&slot)?.unwrap_or(Vec::new()))
    }

    fn get_block_status(&self, id: OLBlockId) -> DbResult<Option<BlockStatus>> {
        Ok(self.blk_status_tree.get(&id)?)
    }

    fn get_tip_block(&self) -> DbResult<OLBlockId> {
        let bht = &self.blk_height_tree;
        let mut slot = bht.last()?.map(first).ok_or(DbError::NotBootstrapped)?;

        loop {
            let blocks = self.get_blocks_at_height(slot)?;
            // Collect all valid blocks at this slot. For OL chain, we expect only one valid block
            // per slot because we are not expecting reorgs. If there are multiple valid blocks
            // (which shouldn't happen), we return the first one.
            let valid = blocks
                .into_iter()
                .filter_map(|blkid| match self.get_block_status(blkid) {
                    Ok(Some(BlockStatus::Valid)) => Some(Ok(blkid)),
                    Ok(_) => None,
                    Err(e) => Some(Err(e)),
                })
                .collect::<Result<Vec<_>, _>>()?;

            // Return the first valid block at the highest slot as the tip.
            // This is fine because we expect only one valid block per slot (no reorgs in OL chain).
            if let Some(id) = valid.first().cloned() {
                return Ok(id);
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

    use super::*;
    use crate::sled_db_test_setup;

    sled_db_test_setup!(OLBlockDBSled, ol_block_db_tests);
}
