//! High-level OL block interface.

use std::sync::Arc;

use strata_db_types::{
    traits::{BlockStatus, OLBlockDatabase},
    DbResult,
};
use strata_identifiers::{OLBlockCommitment, OLBlockId};
use strata_ol_chain_types_new::OLBlock;
use threadpool::ThreadPool;

use crate::{cache, ops};

/// Caching manager of OL blocks in the block database.
#[expect(
    missing_debug_implementations,
    reason = "Some inner types don't have Debug implementation"
)]
pub struct OLBlockManager {
    ops: ops::ol::OLBlockOps,
    block_cache: cache::CacheTable<OLBlockCommitment, Option<OLBlock>>,
}

impl OLBlockManager {
    pub fn new(pool: ThreadPool, db: Arc<impl OLBlockDatabase + 'static>) -> Self {
        let ops = ops::ol::Context::new(db).into_ops(pool);
        let block_cache = cache::CacheTable::new(64.try_into().unwrap());
        Self { ops, block_cache }
    }

    /// Puts a block in the database, purging cache entry.
    pub async fn put_block_data_async(
        &self,
        commitment: OLBlockCommitment,
        block: OLBlock,
    ) -> DbResult<()> {
        self.ops
            .put_block_data_async(commitment, block.clone())
            .await?;
        self.block_cache.purge_async(&commitment).await;
        Ok(())
    }

    /// Puts a block in the database, purging cache entry.
    pub fn put_block_data_blocking(
        &self,
        commitment: OLBlockCommitment,
        block: OLBlock,
    ) -> DbResult<()> {
        self.ops
            .put_block_data_blocking(commitment, block.clone())?;
        self.block_cache.purge_blocking(&commitment);
        Ok(())
    }

    /// Gets a block either in the cache or from the underlying database.
    pub async fn get_block_data_async(
        &self,
        commitment: &OLBlockCommitment,
    ) -> DbResult<Option<OLBlock>> {
        self.block_cache
            .get_or_fetch(commitment, || self.ops.get_block_data_chan(*commitment))
            .await
    }

    /// Gets a block either in the cache or from the underlying database.
    pub fn get_block_data_blocking(
        &self,
        commitment: &OLBlockCommitment,
    ) -> DbResult<Option<OLBlock>> {
        self.block_cache
            .get_or_fetch_blocking(commitment, || self.ops.get_block_data_blocking(*commitment))
    }

    /// Deletes a block from the database, purging cache entry.
    pub async fn del_block_data_async(&self, commitment: OLBlockCommitment) -> DbResult<()> {
        self.ops.del_block_data_async(commitment).await?;
        self.block_cache.purge_async(&commitment).await;
        Ok(())
    }

    /// Deletes a block from the database, purging cache entry.
    pub fn del_block_data_blocking(&self, commitment: OLBlockCommitment) -> DbResult<()> {
        self.ops.del_block_data_blocking(commitment)?;
        self.block_cache.purge_blocking(&commitment);
        Ok(())
    }

    /// Gets the block IDs at a specific slot. Async.
    pub async fn get_blocks_at_height_async(&self, slot: u64) -> DbResult<Vec<OLBlockId>> {
        self.ops.get_blocks_at_height_async(slot).await
    }

    /// Gets the block IDs at a specific slot. Blocking.
    pub fn get_blocks_at_height_blocking(&self, slot: u64) -> DbResult<Vec<OLBlockId>> {
        self.ops.get_blocks_at_height_blocking(slot)
    }

    /// Gets the tip block ID. Async.
    pub async fn get_tip_block_async(&self) -> DbResult<OLBlockId> {
        self.ops.get_tip_block_async().await
    }

    /// Gets the tip block ID. Blocking.
    pub fn get_tip_block_blocking(&self) -> DbResult<OLBlockId> {
        self.ops.get_tip_block_blocking()
    }

    /// Gets the block's verification status. Async.
    pub async fn get_block_status_async(&self, id: &OLBlockId) -> DbResult<Option<BlockStatus>> {
        self.ops.get_block_status_async(*id).await
    }

    /// Gets the block's verification status. Blocking.
    pub fn get_block_status_blocking(&self, id: &OLBlockId) -> DbResult<Option<BlockStatus>> {
        self.ops.get_block_status_blocking(*id)
    }

    /// Sets the block's verification status. Async.
    pub async fn set_block_status_async(
        &self,
        id: &OLBlockId,
        status: BlockStatus,
    ) -> DbResult<()> {
        self.ops.set_block_status_async(*id, status).await?;
        Ok(())
    }

    /// Sets the block's verification status. Blocking.
    pub fn set_block_status_blocking(&self, id: &OLBlockId, status: BlockStatus) -> DbResult<()> {
        self.ops.set_block_status_blocking(*id, status)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use strata_db_store_sled::test_utils::get_test_sled_backend;
    use strata_db_types::traits::{BlockStatus, DatabaseBackend};
    use strata_identifiers::{Buf32, Buf64, OLBlockCommitment, OLBlockId};
    use strata_ol_chain_types_new::{
        BlockFlags, OLBlock, OLBlockBody, OLBlockHeader, OLL1ManifestContainer, OLL1Update,
        OLTxSegment, SignedOLBlockHeader,
    };
    use threadpool::ThreadPool;

    use super::*;

    fn setup_manager() -> OLBlockManager {
        let pool = ThreadPool::new(1);
        let db = Arc::new(get_test_sled_backend());
        let ol_block_db = db.ol_block_db();
        OLBlockManager::new(pool, ol_block_db)
    }

    fn get_mock_block_with_slot(slot: u64) -> OLBlock {
        get_mock_block_with_slot_and_id(slot, 0)
    }

    fn get_mock_block_with_slot_and_id(slot: u64, id_byte: u8) -> OLBlock {
        // Use id_byte to create different body_root to ensure different block IDs
        let mut body_root_bytes = [0u8; 32];
        body_root_bytes[0] = id_byte;
        let body_root = Buf32::from(body_root_bytes);

        let header = OLBlockHeader::new(
            0,                              // timestamp
            BlockFlags::from(0),            // flags
            slot,                           // slot
            0,                              // epoch
            OLBlockId::from(Buf32::zero()), // parent_blkid
            body_root,                      // body_root (varies by id_byte)
            Buf32::zero(),                  // state_root
            Buf32::zero(),                  // logs_root
        );
        let signed_header = SignedOLBlockHeader::new(header, Buf64::zero());
        let body = OLBlockBody {
            tx_segment: Some(OLTxSegment { txs: vec![].into() }).into(),
            l1_update: Some(OLL1Update {
                preseal_state_root: Buf32::zero(),
                manifest_cont: OLL1ManifestContainer::new(vec![])
                    .expect("empty manifest should succeed"),
            })
            .into(),
        };
        OLBlock::new(signed_header, body)
    }

    #[tokio::test]
    async fn test_put_and_get_block_data_async() {
        let manager = setup_manager();
        let block = get_mock_block_with_slot(0);
        let block_id = block.header().compute_blkid();
        let commitment = OLBlockCommitment::new(0u64, block_id);

        // Put block
        manager
            .put_block_data_async(commitment, block.clone())
            .await
            .expect("test: put");

        // Get block (should be cached)
        let retrieved = manager
            .get_block_data_async(&commitment)
            .await
            .expect("test: get")
            .unwrap();
        assert_eq!(
            retrieved.header().compute_blkid(),
            block.header().compute_blkid()
        );
        assert_eq!(retrieved.header().slot(), block.header().slot());
    }

    #[test]
    fn test_put_and_get_block_data_blocking() {
        let manager = setup_manager();
        let block = get_mock_block_with_slot(0);
        let block_id = block.header().compute_blkid();
        let commitment = OLBlockCommitment::new(0u64, block_id);

        // Put block
        manager
            .put_block_data_blocking(commitment, block.clone())
            .expect("test: put");

        // Get block (should be cached)
        let retrieved = manager
            .get_block_data_blocking(&commitment)
            .expect("test: get")
            .unwrap();
        assert_eq!(
            retrieved.header().compute_blkid(),
            block.header().compute_blkid()
        );
        assert_eq!(retrieved.header().slot(), block.header().slot());
    }

    #[tokio::test]
    async fn test_get_blocks_at_height_async() {
        let manager = setup_manager();
        let slot = 10u64;
        let block1 = get_mock_block_with_slot_and_id(slot, 1);
        let block_id1 = block1.header().compute_blkid();
        let commitment1 = OLBlockCommitment::new(slot, block_id1);

        let block2 = get_mock_block_with_slot_and_id(slot, 2);
        let block_id2 = block2.header().compute_blkid();
        let commitment2 = OLBlockCommitment::new(slot, block_id2);

        // Put two blocks at the same slot
        manager
            .put_block_data_async(commitment1, block1)
            .await
            .expect("test: put block 1");
        manager
            .put_block_data_async(commitment2, block2)
            .await
            .expect("test: put block 2");

        // Get blocks at height
        let block_ids = manager
            .get_blocks_at_height_async(slot)
            .await
            .expect("test: get blocks at height");
        assert_eq!(block_ids.len(), 2);
        assert!(block_ids.contains(&block_id1));
        assert!(block_ids.contains(&block_id2));
    }

    #[test]
    fn test_get_blocks_at_height_blocking() {
        let manager = setup_manager();
        let slot = 10u64;
        let block1 = get_mock_block_with_slot_and_id(slot, 1);
        let block_id1 = block1.header().compute_blkid();
        let commitment1 = OLBlockCommitment::new(slot, block_id1);

        let block2 = get_mock_block_with_slot_and_id(slot, 2);
        let block_id2 = block2.header().compute_blkid();
        let commitment2 = OLBlockCommitment::new(slot, block_id2);

        // Put two blocks at the same slot
        manager
            .put_block_data_blocking(commitment1, block1)
            .expect("test: put block 1");
        manager
            .put_block_data_blocking(commitment2, block2)
            .expect("test: put block 2");

        // Get blocks at height
        let block_ids = manager
            .get_blocks_at_height_blocking(slot)
            .expect("test: get blocks at height");
        assert_eq!(block_ids.len(), 2);
        assert!(block_ids.contains(&block_id1));
        assert!(block_ids.contains(&block_id2));
    }

    #[tokio::test]
    async fn test_set_and_get_block_status_async() {
        let manager = setup_manager();
        let block = get_mock_block_with_slot(0);
        let block_id = block.header().compute_blkid();
        let commitment = OLBlockCommitment::new(0u64, block_id);

        // Put block
        manager
            .put_block_data_async(commitment, block)
            .await
            .expect("test: put");

        // Set and get status
        manager
            .set_block_status_async(&block_id, BlockStatus::Valid)
            .await
            .expect("test: set status");
        let status = manager
            .get_block_status_async(&block_id)
            .await
            .expect("test: get status")
            .unwrap();
        assert_eq!(status, BlockStatus::Valid);
    }

    #[test]
    fn test_set_and_get_block_status_blocking() {
        let manager = setup_manager();
        let block = get_mock_block_with_slot(0);
        let block_id = block.header().compute_blkid();
        let commitment = OLBlockCommitment::new(0u64, block_id);

        // Put block
        manager
            .put_block_data_blocking(commitment, block)
            .expect("test: put");

        // Set and get status
        manager
            .set_block_status_blocking(&block_id, BlockStatus::Valid)
            .expect("test: set status");
        let status = manager
            .get_block_status_blocking(&block_id)
            .expect("test: get status")
            .unwrap();
        assert_eq!(status, BlockStatus::Valid);
    }

    #[tokio::test]
    async fn test_get_tip_block_async() {
        let manager = setup_manager();
        let block1 = get_mock_block_with_slot(5u64);
        let block_id1 = block1.header().compute_blkid();
        let commitment1 = OLBlockCommitment::new(5u64, block_id1);

        let block2 = get_mock_block_with_slot(10u64);
        let block_id2 = block2.header().compute_blkid();
        let commitment2 = OLBlockCommitment::new(10u64, block_id2);

        // Put blocks
        manager
            .put_block_data_async(commitment1, block1)
            .await
            .expect("test: put block 1");
        manager
            .put_block_data_async(commitment2, block2)
            .await
            .expect("test: put block 2");

        // Set block2 as valid (higher slot)
        manager
            .set_block_status_async(&block_id2, BlockStatus::Valid)
            .await
            .expect("test: set block 2 status");

        // Get tip block - should be block2 (highest valid slot)
        let tip = manager
            .get_tip_block_async()
            .await
            .expect("test: get tip block");
        assert_eq!(tip, block_id2);
    }

    #[test]
    fn test_get_tip_block_blocking() {
        let manager = setup_manager();
        let block1 = get_mock_block_with_slot(5u64);
        let block_id1 = block1.header().compute_blkid();
        let commitment1 = OLBlockCommitment::new(5u64, block_id1);

        let block2 = get_mock_block_with_slot(10u64);
        let block_id2 = block2.header().compute_blkid();
        let commitment2 = OLBlockCommitment::new(10u64, block_id2);

        // Put blocks
        manager
            .put_block_data_blocking(commitment1, block1)
            .expect("test: put block 1");
        manager
            .put_block_data_blocking(commitment2, block2)
            .expect("test: put block 2");

        // Set block2 as valid (higher slot)
        manager
            .set_block_status_blocking(&block_id2, BlockStatus::Valid)
            .expect("test: set block 2 status");

        // Get tip block - should be block2 (highest valid slot)
        let tip = manager
            .get_tip_block_blocking()
            .expect("test: get tip block");
        assert_eq!(tip, block_id2);
    }
}
