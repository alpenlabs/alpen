use std::sync::Arc;

use strata_db::{
    DbError, DbResult,
    traits::{BlockStatus, L2BlockDatabase},
};
use strata_state::{block::L2BlockBundle, header::L2Header, id::L2BlockId};
use typed_sled::{SledDb, SledTree};

use crate::{
    SledDbConfig,
    l2::schemas::{L2BlockHeightSchema, L2BlockSchema, L2BlockStatusSchema},
    utils::first,
};

#[derive(Debug)]
pub struct L2DBSled {
    blk_tree: SledTree<L2BlockSchema>,
    blk_status_tree: SledTree<L2BlockStatusSchema>,
    blk_height_tree: SledTree<L2BlockHeightSchema>,
    config: SledDbConfig,
}

impl L2DBSled {
    pub fn new(db: Arc<SledDb>, config: SledDbConfig) -> DbResult<Self> {
        Ok(Self {
            blk_tree: db.get_tree()?,
            blk_status_tree: db.get_tree()?,
            blk_height_tree: db.get_tree()?,
            config,
        })
    }
}

impl L2BlockDatabase for L2DBSled {
    fn put_block_data(&self, bundle: L2BlockBundle) -> DbResult<()> {
        let block_id = bundle.block().header().get_blockid();
        let block_height = bundle.block().header().slot();

        self.config
            .with_retry(
                (&self.blk_tree, &self.blk_status_tree, &self.blk_height_tree),
                |(bt, bst, bht)| {
                    let mut block_height_data = bht.get(&block_height)?.unwrap_or(Vec::new());
                    if !block_height_data.contains(&block_id) {
                        block_height_data.push(block_id);
                    }

                    bt.insert(&block_id, &bundle)?;
                    bst.insert(&block_id, &BlockStatus::Unchecked)?;
                    bht.insert(&block_height, &block_height_data)?;
                    Ok(())
                },
            )
            .map_err(|e| DbError::Other(e.to_string()))?;
        Ok(())
    }

    fn del_block_data(&self, id: L2BlockId) -> DbResult<bool> {
        let bundle = match self.get_block_data(id)? {
            Some(block) => block,
            None => return Ok(false),
        };

        let block_height = bundle.block().header().slot();

        self.config
            .with_retry(
                (&self.blk_tree, &self.blk_status_tree, &self.blk_height_tree),
                |(bt, bst, bht)| {
                    let mut block_height_data = bht.get(&block_height)?.unwrap_or(Vec::new());
                    block_height_data.retain(|&block_id| block_id != id);

                    bt.remove(&id)?;
                    bst.remove(&id)?;
                    bht.insert(&block_height, &block_height_data)?;

                    Ok(true)
                },
            )
            .map_err(|e| DbError::Other(e.to_string()))
    }

    fn set_block_status(&self, id: L2BlockId, status: BlockStatus) -> DbResult<()> {
        if self.get_block_data(id)?.is_none() {
            return Ok(());
        }
        Ok(self.blk_status_tree.insert(&id, &status)?)
    }

    fn get_block_data(&self, id: L2BlockId) -> DbResult<Option<L2BlockBundle>> {
        Ok(self.blk_tree.get(&id)?)
    }

    fn get_blocks_at_height(&self, idx: u64) -> DbResult<Vec<L2BlockId>> {
        Ok(self.blk_height_tree.get(&idx)?.unwrap_or(Vec::new()))
    }

    fn get_block_status(&self, id: L2BlockId) -> DbResult<Option<BlockStatus>> {
        Ok(self.blk_status_tree.get(&id)?)
    }

    fn get_tip_block(&self) -> DbResult<L2BlockId> {
        let bht = &self.blk_height_tree;
        let mut height = bht.last()?.map(first).ok_or(DbError::NotBootstrapped)?;

        loop {
            let blocks = self.get_blocks_at_height(height)?;
            // collect all valid statuses at this height
            let valid = blocks
                .into_iter()
                .filter_map(|blkid| match self.get_block_status(blkid) {
                    Ok(Some(BlockStatus::Valid)) => Some(Ok(blkid)),
                    Ok(_) => None,
                    Err(e) => Some(Err(e)),
                })
                .collect::<Result<Vec<_>, _>>()?;

            // Return the first valid block at the highest height as the tip.
            if let Some(id) = valid.first().cloned() {
                return Ok(id);
            }

            if height == 0 {
                return Err(DbError::NotBootstrapped);
            }

            height -= 1;
        }
    }
}

#[cfg(feature = "test_utils")]
#[cfg(test)]
mod tests {
    use strata_db_tests::l2_db_tests;

    use super::*;

    fn setup_db() -> L2DBSled {
        let db = sled::Config::new().temporary(true).open().unwrap();
        let sled_db = SledDb::new(db).unwrap();
        let config = SledDbConfig::new_with_constant_backoff(3, 200);
        L2DBSled::new(sled_db.into(), config).unwrap()
    }

    l2_db_tests!(setup_db());
}
