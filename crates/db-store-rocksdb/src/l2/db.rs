use std::sync::Arc;

use rockbound::{OptimisticTransactionDB, SchemaDBOperationsExt};
use strata_db::{
    errors::DbError,
    traits::{BlockStatus, L2BlockDatabase},
    DbResult,
};
use strata_state::{block::L2BlockBundle, prelude::*};

use super::schemas::{L2BlockSchema, L2BlockStatusSchema};
use crate::{l2::schemas::L2BlockHeightSchema, DbOpsConfig};

#[derive(Debug)]
pub struct L2Db {
    db: Arc<OptimisticTransactionDB>,
    ops: DbOpsConfig,
}

impl L2Db {
    pub fn new(db: Arc<OptimisticTransactionDB>, ops: DbOpsConfig) -> Self {
        Self { db, ops }
    }
}

impl L2BlockDatabase for L2Db {
    fn put_block_data(&self, bundle: L2BlockBundle) -> DbResult<()> {
        let block_id = bundle.block().header().get_blockid();

        // append to previous block height data
        let block_height = bundle.block().header().slot();

        self.db
            .with_optimistic_txn(
                rockbound::TransactionRetry::Count(self.ops.retry_count),
                |txn| {
                    let mut block_height_data = txn
                        .get_for_update::<L2BlockHeightSchema>(&block_height)?
                        .unwrap_or(Vec::new());
                    if !block_height_data.contains(&block_id) {
                        block_height_data.push(block_id);
                    }

                    txn.put::<L2BlockSchema>(&block_id, &bundle)?;
                    txn.put::<L2BlockStatusSchema>(&block_id, &BlockStatus::Unchecked)?;
                    txn.put::<L2BlockHeightSchema>(&block_height, &block_height_data)?;

                    Ok::<_, anyhow::Error>(())
                },
            )
            .map_err(|e| DbError::TransactionError(e.to_string()))
    }

    fn del_block_data(&self, id: L2BlockId) -> DbResult<bool> {
        let bundle = match self.get_block_data(id)? {
            Some(block) => block,
            None => return Ok(false),
        };

        // update to previous block height data
        let block_height = bundle.block().header().slot();
        let mut block_height_data = self.get_blocks_at_height(block_height)?;
        block_height_data.retain(|&block_id| block_id != id);

        self.db
            .with_optimistic_txn(
                rockbound::TransactionRetry::Count(self.ops.retry_count),
                |txn| {
                    let mut block_height_data = txn
                        .get_for_update::<L2BlockHeightSchema>(&block_height)?
                        .unwrap_or(Vec::new());
                    block_height_data.retain(|&block_id| block_id != id);

                    txn.delete::<L2BlockSchema>(&id)?;
                    txn.delete::<L2BlockStatusSchema>(&id)?;
                    txn.put::<L2BlockHeightSchema>(&block_height, &block_height_data)?;

                    Ok::<_, anyhow::Error>(true)
                },
            )
            .map_err(|e| DbError::TransactionError(e.to_string()))
    }

    fn set_block_status(&self, id: L2BlockId, status: BlockStatus) -> DbResult<()> {
        if self.get_block_data(id)?.is_none() {
            return Ok(());
        }
        self.db.put::<L2BlockStatusSchema>(&id, &status)?;

        Ok(())
    }

    fn get_block_data(&self, id: L2BlockId) -> DbResult<Option<L2BlockBundle>> {
        Ok(self.db.get::<L2BlockSchema>(&id)?)
    }

    fn get_blocks_at_height(&self, idx: u64) -> DbResult<Vec<L2BlockId>> {
        Ok(self
            .db
            .get::<L2BlockHeightSchema>(&idx)?
            .unwrap_or(Vec::new()))
    }

    fn get_block_status(&self, id: L2BlockId) -> DbResult<Option<BlockStatus>> {
        Ok(self.db.get::<L2BlockStatusSchema>(&id)?)
    }

    fn get_tip_block(&self) -> DbResult<L2BlockId> {
        use crate::utils::{get_last_idx};
        
        let mut height = get_last_idx::<L2BlockHeightSchema>(&self.db)?.ok_or(DbError::NotBootstrapped)?;

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
    use crate::test_utils::get_rocksdb_tmp_instance;

    fn setup_db() -> L2Db {
        let (db, ops) = get_rocksdb_tmp_instance().unwrap();
        L2Db::new(db, ops)
    }

    l2_db_tests!(setup_db());
}
