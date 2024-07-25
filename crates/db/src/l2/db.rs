use std::sync::Arc;

use rockbound::{SchemaBatch, DB};

use alpen_vertex_state::prelude::*;

use crate::{
    l2::schemas::L2BlockHeightSchema,
    traits::{BlockStatus, L2DataProvider, L2DataStore},
    DbResult,
};

use super::schemas::{L2BlockSchema, L2BlockStatusSchema};

pub struct L2Db {
    db: Arc<DB>,
}

impl L2Db {
    pub fn new(db: Arc<DB>) -> Self {
        Self { db }
    }
}

impl L2DataStore for L2Db {
    fn put_block_data(&self, block: L2Block) -> DbResult<()> {
        let block_id = block.header().get_blockid();

        // append to previous block height data
        let block_height = block.header().blockidx();
        let mut block_height_data = self.get_blocks_at_height(block_height)?;
        if !block_height_data.contains(&block_id) {
            block_height_data.push(block_id);
        }

        let mut batch = SchemaBatch::new();
        batch.put::<L2BlockSchema>(&block_id, &block)?;
        batch.put::<L2BlockStatusSchema>(&block_id, &BlockStatus::Unchecked)?;
        batch.put::<L2BlockHeightSchema>(&block_height, &block_height_data)?;
        self.db.write_schemas(batch)?;

        Ok(())
    }

    fn del_block_data(&self, id: L2BlockId) -> DbResult<bool> {
        let block = match self.get_block_data(id)? {
            Some(block) => block,
            None => return Ok(false),
        };

        // update to previous block height data
        let block_height = block.header().blockidx();
        let mut block_height_data = self.get_blocks_at_height(block_height)?;
        block_height_data.retain(|&block_id| block_id != id);

        let mut batch = SchemaBatch::new();
        batch.delete::<L2BlockSchema>(&id)?;
        batch.delete::<L2BlockStatusSchema>(&id)?;
        batch.put::<L2BlockHeightSchema>(&block_height, &block_height_data)?;
        self.db.write_schemas(batch)?;

        Ok(true)
    }

    fn set_block_status(&self, id: L2BlockId, status: BlockStatus) -> DbResult<()> {
        if self.get_block_data(id)?.is_none() {
            return Ok(());
        }

        let mut batch = SchemaBatch::new();
        batch.put::<L2BlockStatusSchema>(&id, &status)?;
        self.db.write_schemas(batch)?;

        Ok(())
    }
}

impl L2DataProvider for L2Db {
    fn get_block_data(&self, id: L2BlockId) -> DbResult<Option<L2Block>> {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use alpen_test_utils::{get_rocksdb_tmp_instance, ArbitraryGenerator};

    fn get_mock_data() -> L2Block {
        let arb = ArbitraryGenerator::new();
        let l2_lock: L2Block = arb.generate();

        l2_lock
    }

    fn setup_db() -> L2Db {
        let db = get_rocksdb_tmp_instance().unwrap();
        L2Db::new(db)
    }

    #[test]
    fn set_and_get_block_data() {
        let l2_db = setup_db();

        let block = get_mock_data();
        let block_hash = block.header().get_blockid();
        let block_height = block.header().blockidx();

        l2_db
            .put_block_data(block.clone())
            .expect("failed to put block data");

        // assert block was stored
        let received_block = l2_db
            .get_block_data(block_hash)
            .expect("failed to retrieve block data")
            .unwrap();
        assert_eq!(received_block, block);

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
        let block = get_mock_data();
        let block_hash = block.header().get_blockid();
        let block_height = block.header().blockidx();

        // deleting non existing block should return false
        let res = l2_db
            .del_block_data(block_hash)
            .expect("failed to remove the block");
        assert!(!res);

        // deleting existing block should return true
        l2_db
            .put_block_data(block.clone())
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
        let block = get_mock_data();
        let block_hash = block.header().get_blockid();

        l2_db
            .put_block_data(block.clone())
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
}
