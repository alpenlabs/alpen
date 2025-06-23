use std::sync::Arc;

use rockbound::{OptimisticTransactionDB, SchemaDBOperationsExt};
use strata_db::{
    errors::DbError,
    traits::{BlockStatus, L2BlockDatabase},
    DbResult,
};
use strata_state::{block::L2BlockBundle, prelude::*};

use super::schemas::{L2BlockSchema, L2BlockStatusSchema};
use crate::{l2::schemas::L2BlockHeightSchema, utils::get_last_idx, DbOpsConfig};

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

    fn get_tip_block(&self) -> DbResult<Option<L2BlockId>> {
        let mut height = get_last_idx::<L2BlockHeightSchema>(&self.db)?.unwrap_or(0);

        while height > 0 {
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

            if !valid.is_empty() {
                // REVIEW: We consider the first valid block at the highest height as the tip.
                // This may not be the best approach but have been used other places in the
                // codebase.
                return Ok(valid.first().cloned());
            }

            height -= 1;
        }

        Ok(None)
    }
}

#[cfg(feature = "test_utils")]
#[cfg(test)]
mod tests {
    use strata_test_utils::ArbitraryGenerator;

    use super::*;
    use crate::test_utils::get_rocksdb_tmp_instance;

    fn get_mock_data() -> L2BlockBundle {
        let mut arb = ArbitraryGenerator::new_with_size(1 << 14);
        let l2_block: L2BlockBundle = arb.generate();

        l2_block
    }

    fn setup_db() -> L2Db {
        let (db, ops) = get_rocksdb_tmp_instance().unwrap();
        L2Db::new(db, ops)
    }

    #[test]
    fn set_and_get_block_data() {
        let l2_db = setup_db();

        let bundle = get_mock_data();
        let block_hash = bundle.block().header().get_blockid();
        let block_height = bundle.block().header().slot();

        l2_db
            .put_block_data(bundle.clone())
            .expect("failed to put block data");

        // assert block was stored
        let received_block = l2_db
            .get_block_data(block_hash)
            .expect("failed to retrieve block data")
            .unwrap();
        assert_eq!(received_block, bundle);

        // assert block status was set to `BlockStatus::Unchecked``
        let block_status = l2_db
            .get_block_status(block_hash)
            .expect("failed to retrieve block data")
            .unwrap();
        assert_eq!(block_status, BlockStatus::Unchecked);

        // assert block height data was stored
        let block_ids = l2_db
            .get_blocks_at_height(block_height)
            .expect("failed to retrieve block data");
        assert!(block_ids.contains(&block_hash))
    }

    #[test]
    fn del_and_get_block_data() {
        let l2_db = setup_db();
        let bundle = get_mock_data();
        let block_hash = bundle.block().header().get_blockid();
        let block_height = bundle.block().header().slot();

        // deleting non existing block should return false
        let res = l2_db
            .del_block_data(block_hash)
            .expect("failed to remove the block");
        assert!(!res);

        // deleting existing block should return true
        l2_db
            .put_block_data(bundle.clone())
            .expect("failed to put block data");
        let res = l2_db
            .del_block_data(block_hash)
            .expect("failed to remove the block");
        assert!(res);

        // assert block is deleted from the db
        let received_block = l2_db
            .get_block_data(block_hash)
            .expect("failed to retrieve block data");
        assert!(received_block.is_none());

        // assert block status is deleted from the db
        let block_status = l2_db
            .get_block_status(block_hash)
            .expect("failed to retrieve block status");
        assert!(block_status.is_none());

        // assert block height data is deleted
        let block_ids = l2_db
            .get_blocks_at_height(block_height)
            .expect("failed to retrieve block data");
        assert!(!block_ids.contains(&block_hash))
    }

    #[test]
    fn set_and_get_block_status() {
        let l2_db = setup_db();
        let bundle = get_mock_data();
        let block_hash = bundle.block().header().get_blockid();

        l2_db
            .put_block_data(bundle.clone())
            .expect("failed to put block data");

        // assert block status was set to `BlockStatus::Valid``
        l2_db
            .set_block_status(block_hash, BlockStatus::Valid)
            .expect("failed to update block status");
        let block_status = l2_db
            .get_block_status(block_hash)
            .expect("failed to retrieve block status")
            .unwrap();
        assert_eq!(block_status, BlockStatus::Valid);

        // assert block status was set to `BlockStatus::Invalid``
        l2_db
            .set_block_status(block_hash, BlockStatus::Invalid)
            .expect("failed to update block status");
        let block_status = l2_db
            .get_block_status(block_hash)
            .expect("failed to retrieve block status")
            .unwrap();
        assert_eq!(block_status, BlockStatus::Invalid);

        // assert block status was set to `BlockStatus::Unchecked``
        l2_db
            .set_block_status(block_hash, BlockStatus::Unchecked)
            .expect("failed to update block status");
        let block_status = l2_db
            .get_block_status(block_hash)
            .expect("failed to retrieve block status")
            .unwrap();
        assert_eq!(block_status, BlockStatus::Unchecked);
    }

    #[test]
    fn get_valid_tip_block_ids_not_bootstrapped() {
        let l2_db = setup_db();
        let err = l2_db.get_tip_block().unwrap_err();
        assert!(matches!(err, DbError::NotBootstrapped));
    }

    // helper to create a bundle at a specific height
    fn make_bundle_at_height(height: u64) -> L2BlockBundle {
        let bundle = get_mock_data();
        let (l2block, acc) = bundle.into_parts();
        let (signed_header, body) = l2block.into_parts();
        let old_hdr = signed_header.header().clone();
        let new_hdr = L2BlockHeader::new(
            height,
            old_hdr.epoch(),
            old_hdr.timestamp(),
            *old_hdr.parent(),
            &body,
            *old_hdr.state_root(),
        );
        let new_signed = SignedL2BlockHeader::new(new_hdr, *signed_header.sig());
        let new_block = L2Block::new(new_signed, body);
        L2BlockBundle::new(new_block, acc)
    }

    #[test]
    fn get_valid_tip_block_ids_fallback_to_lower_height() {
        let l2_db = setup_db();

        // tip at height 10 with unchecked status
        let tip_bundle = make_bundle_at_height(10);
        l2_db.put_block_data(tip_bundle).unwrap();

        // lower at height 9 marked valid
        let lower_bundle = make_bundle_at_height(9);
        let lower_id = lower_bundle.block().header().get_blockid();
        l2_db.put_block_data(lower_bundle).unwrap();
        l2_db
            .set_block_status(lower_id, BlockStatus::Valid)
            .unwrap();

        // should skip height 10 and return the valid lower block
        let valid = l2_db.get_tip_block().unwrap();
        assert_eq!(valid, Some(lower_id));
    }

    #[test]
    fn get_valid_tip_block_ids_empty_if_no_valid_any_height() {
        let l2_db = setup_db();

        // insert bundles at heights 5 and 6, leave both unchecked
        let b1 = make_bundle_at_height(5);
        let b2 = make_bundle_at_height(6);
        l2_db.put_block_data(b1).unwrap();
        l2_db.put_block_data(b2).unwrap();

        let valid = l2_db.get_tip_block().unwrap();
        assert!(valid.is_none());
    }
}
