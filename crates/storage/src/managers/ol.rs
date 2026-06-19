//! High-level OL block interface.

use std::sync::Arc;

use strata_db_types::{
    traits::{BlockStatus, OLBlockDatabase},
    DbResult,
};
use strata_identifiers::{OLBlockId, Slot};
use strata_ol_chain_types_new::OLBlock;
use strata_primitives::OLBlockCommitment;
use threadpool::ThreadPool;

use crate::{cache, ops};

/// Caching manager of OL blocks in the block database.
#[expect(
    missing_debug_implementations,
    reason = "Some inner types don't have Debug implementation"
)]
pub struct OLBlockManager {
    ops: ops::ol::OLBlockOps,
    block_cache: cache::CacheTable<OLBlockId, Option<OLBlock>>,
}

impl OLBlockManager {
    pub fn new(pool: ThreadPool, db: Arc<impl OLBlockDatabase + 'static>) -> Self {
        let ops = ops::ol::Context::new(db).into_ops(pool);
        let block_cache = cache::CacheTable::new(64.try_into().unwrap());
        Self { ops, block_cache }
    }

    /// Puts a block in the database, purging cache entry.
    pub async fn put_block_data_async(&self, block: OLBlock) -> DbResult<()> {
        let block_id = block.header().compute_blkid();
        self.ops.put_block_data_async(block).await?;
        self.block_cache.purge_async(&block_id).await;
        Ok(())
    }

    /// Puts a block in the database, purging cache entry.
    pub fn put_block_data_blocking(&self, block: OLBlock) -> DbResult<()> {
        let block_id = block.header().compute_blkid();
        self.ops.put_block_data_blocking(block)?;
        self.block_cache.purge_blocking(&block_id);
        Ok(())
    }

    /// Gets the block high-watermark.
    ///
    /// This is not the OL tip. Plain block writes do not update it.
    pub async fn get_block_high_watermark_async(&self) -> DbResult<Option<OLBlockCommitment>> {
        self.ops.get_block_high_watermark_async().await
    }

    /// Gets the block high-watermark.
    ///
    /// This is not the OL tip. Plain block writes do not update it.
    pub fn get_block_high_watermark_blocking(&self) -> DbResult<Option<OLBlockCommitment>> {
        self.ops.get_block_high_watermark_blocking()
    }

    /// Puts a block and advances the block high-watermark atomically.
    ///
    /// This uses the guarded high-watermark path; plain block writes do not update it.
    pub async fn put_block_data_with_high_watermark_async(
        &self,
        block: OLBlock,
    ) -> DbResult<OLBlockCommitment> {
        let block_id = block.header().compute_blkid();
        let commitment = self
            .ops
            .put_block_data_with_high_watermark_async(block)
            .await?;
        self.block_cache.purge_async(&block_id).await;
        Ok(commitment)
    }

    /// Puts a block and advances the block high-watermark atomically.
    ///
    /// This uses the guarded high-watermark path; plain block writes do not update it.
    pub fn put_block_data_with_high_watermark_blocking(
        &self,
        block: OLBlock,
    ) -> DbResult<OLBlockCommitment> {
        let block_id = block.header().compute_blkid();
        let commitment = self
            .ops
            .put_block_data_with_high_watermark_blocking(block)?;
        self.block_cache.purge_blocking(&block_id);
        Ok(commitment)
    }

    /// Clears the block high-watermark if it currently equals `expected`.
    ///
    /// This does not delete block data or purge cached blocks.
    pub async fn clear_block_high_watermark_async(
        &self,
        expected: OLBlockCommitment,
    ) -> DbResult<bool> {
        self.ops.clear_block_high_watermark_async(expected).await
    }

    /// Clears the block high-watermark if it currently equals `expected`.
    ///
    /// This does not delete block data or purge cached blocks.
    pub fn clear_block_high_watermark_blocking(
        &self,
        expected: OLBlockCommitment,
    ) -> DbResult<bool> {
        self.ops.clear_block_high_watermark_blocking(expected)
    }

    /// Rolls the block high-watermark back to an existing target block.
    ///
    /// This is not normal chain progression; it is used by explicit recovery
    /// paths that revert OL state.
    pub async fn rollback_block_high_watermark_async(
        &self,
        target: OLBlockCommitment,
    ) -> DbResult<bool> {
        self.ops.rollback_block_high_watermark_async(target).await
    }

    /// Rolls the block high-watermark back to an existing target block.
    ///
    /// This is not normal chain progression; it is used by explicit recovery
    /// paths that revert OL state.
    pub fn rollback_block_high_watermark_blocking(
        &self,
        target: OLBlockCommitment,
    ) -> DbResult<bool> {
        self.ops.rollback_block_high_watermark_blocking(target)
    }

    /// Gets a block either in the cache or from the underlying database.
    pub async fn get_block_data_async(&self, id: OLBlockId) -> DbResult<Option<OLBlock>> {
        self.block_cache
            .get_or_fetch(&id, || self.ops.get_block_data_chan(id))
            .await
    }

    /// Gets a block either in the cache or from the underlying database.
    pub fn get_block_data_blocking(&self, id: OLBlockId) -> DbResult<Option<OLBlock>> {
        self.block_cache
            .get_or_fetch_blocking(&id, || self.ops.get_block_data_blocking(id))
    }

    /// Deletes a block from the database, purging cache entry.
    /// Returns true if the block existed and was deleted.
    pub async fn del_block_data_async(&self, id: OLBlockId) -> DbResult<bool> {
        let existed = self.ops.del_block_data_async(id).await?;
        if existed {
            self.block_cache.purge_async(&id).await;
        }
        Ok(existed)
    }

    /// Deletes a block from the database, purging cache entry.
    /// Returns true if the block existed and was deleted.
    pub fn del_block_data_blocking(&self, id: OLBlockId) -> DbResult<bool> {
        let existed = self.ops.del_block_data_blocking(id)?;
        if existed {
            self.block_cache.purge_blocking(&id);
        }
        Ok(existed)
    }

    /// Gets the block IDs at a specific slot. Async.
    pub async fn get_blocks_at_height_async(&self, slot: u64) -> DbResult<Vec<OLBlockId>> {
        self.ops.get_blocks_at_height_async(slot).await
    }

    /// Gets the block IDs at a specific slot. Blocking.
    pub fn get_blocks_at_height_blocking(&self, slot: u64) -> DbResult<Vec<OLBlockId>> {
        self.ops.get_blocks_at_height_blocking(slot)
    }

    /// Gets the canonical tip slot. Async.
    pub async fn get_tip_slot_async(&self) -> DbResult<Slot> {
        self.ops.get_tip_slot_async().await
    }

    /// Gets the canonical tip slot. Blocking.
    pub fn get_tip_slot_blocking(&self) -> DbResult<Slot> {
        self.ops.get_tip_slot_blocking()
    }

    /// Gets the block's verification status. Async.
    pub async fn get_block_status_async(&self, id: OLBlockId) -> DbResult<Option<BlockStatus>> {
        self.ops.get_block_status_async(id).await
    }

    /// Gets the block's verification status. Blocking.
    pub fn get_block_status_blocking(&self, id: OLBlockId) -> DbResult<Option<BlockStatus>> {
        self.ops.get_block_status_blocking(id)
    }

    /// Sets the block's verification status. Returns true if the status was updated. Async.
    pub async fn set_block_status_async(
        &self,
        id: OLBlockId,
        status: BlockStatus,
    ) -> DbResult<bool> {
        self.ops.set_block_status_async(id, status).await
    }

    /// Sets the block's verification status. Returns true if the status was updated. Blocking.
    pub fn set_block_status_blocking(&self, id: OLBlockId, status: BlockStatus) -> DbResult<bool> {
        self.ops.set_block_status_blocking(id, status)
    }

    /// Gets the canonical tip block commitment.
    pub fn get_canonical_tip_blocking(&self) -> DbResult<Option<OLBlockCommitment>> {
        let tip = self.get_tip_slot_blocking()?;
        self.get_canonical_block_at_blocking(tip)
    }

    /// Gets the canonical tip block commitment.
    pub async fn get_canonical_tip_async(&self) -> DbResult<Option<OLBlockCommitment>> {
        let tip = self.get_tip_slot_async().await?;
        self.get_canonical_block_at_async(tip).await
    }

    /// Gets the canonical block commitment at given slot. Blocking.
    ///
    /// Reads the canonical index recorded by fork choice. Returns `None` for slots above the
    /// canonical tip or not yet written.
    pub fn get_canonical_block_at_blocking(
        &self,
        slot: Slot,
    ) -> DbResult<Option<OLBlockCommitment>> {
        Ok(self
            .ops
            .get_canonical_block_blocking(slot)?
            .map(|id| OLBlockCommitment::new(slot, id)))
    }

    /// Gets the canonical block commitment at given slot. Async.
    ///
    /// Reads the canonical index recorded by fork choice. Returns `None` for slots above the
    /// canonical tip or not yet written.
    pub async fn get_canonical_block_at_async(
        &self,
        slot: Slot,
    ) -> DbResult<Option<OLBlockCommitment>> {
        Ok(self
            .ops
            .get_canonical_block_async(slot)
            .await?
            .map(|id| OLBlockCommitment::new(slot, id)))
    }

    /// Replaces the canonical suffix from `start_slot`.
    ///
    /// Atomically removes every canonical entry for slots greater than or equal to `start_slot`,
    /// then writes each block ID into a contiguous suffix starting at `start_slot`.
    pub async fn replace_canonical_suffix_from_async(
        &self,
        start_slot: Slot,
        block_ids: Vec<OLBlockId>,
    ) -> DbResult<()> {
        self.ops
            .replace_canonical_suffix_from_async(start_slot, block_ids)
            .await
    }

    /// Replaces the canonical suffix from `start_slot`.
    ///
    /// Atomically removes every canonical entry for slots greater than or equal to `start_slot`,
    /// then writes each block ID into a contiguous suffix starting at `start_slot`.
    pub fn replace_canonical_suffix_from_blocking(
        &self,
        start_slot: Slot,
        block_ids: Vec<OLBlockId>,
    ) -> DbResult<()> {
        self.ops
            .replace_canonical_suffix_from_blocking(start_slot, block_ids)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use proptest::prelude::*;
    use strata_db_store_sled::test_utils::get_test_sled_backend;
    use strata_db_types::traits::{BlockStatus, DatabaseBackend};
    use strata_identifiers::{Buf32, OLBlockId};
    use strata_ol_chain_types_new::test_utils as ol_test_utils;
    use threadpool::ThreadPool;
    use tokio::runtime::Runtime;

    use super::*;

    fn setup_manager() -> OLBlockManager {
        let pool = ThreadPool::new(1);
        let db = Arc::new(get_test_sled_backend());
        let ol_block_db = db.ol_block_db();
        OLBlockManager::new(pool, ol_block_db)
    }

    proptest! {
        #[test]
        fn proptest_put_and_get_block_data_async(block in ol_test_utils::ol_block_strategy()) {
            let rt = Runtime::new().unwrap();
            rt.block_on(async {
                let manager = setup_manager();
                let block_id = block.header().compute_blkid();

                manager.put_block_data_async(block.clone()).await.expect("put block");

                let retrieved = manager
                    .get_block_data_async(block_id)
                    .await
                    .expect("get block")
                    .unwrap();
                assert_eq!(retrieved.header().compute_blkid(), block.header().compute_blkid());
                assert_eq!(retrieved.header().slot(), block.header().slot());
            });
        }

        #[test]
        fn proptest_put_and_get_block_data_blocking(block in ol_test_utils::ol_block_strategy()) {
            let manager = setup_manager();
            let block_id = block.header().compute_blkid();

            manager.put_block_data_blocking(block.clone()).expect("put block");

            let retrieved = manager
                .get_block_data_blocking(block_id)
                .expect("get block")
                .unwrap();
            assert_eq!(retrieved.header().compute_blkid(), block.header().compute_blkid());
            assert_eq!(retrieved.header().slot(), block.header().slot());
        }

        #[test]
        fn proptest_status_transitions_async(block in ol_test_utils::ol_block_strategy()) {
            let rt = Runtime::new().unwrap();
            rt.block_on(async {
                let manager = setup_manager();
                let block_id = block.header().compute_blkid();

                manager.put_block_data_async(block.clone()).await.expect("put block");

                // Set to Valid
                manager.set_block_status_async(block_id, BlockStatus::Valid).await.expect("set valid");
                let status = manager.get_block_status_async(block_id).await.expect("get status").unwrap();
                assert_eq!(status, BlockStatus::Valid);

                // Set to Invalid
                manager.set_block_status_async(block_id, BlockStatus::Invalid).await.expect("set invalid");
                let status = manager.get_block_status_async(block_id).await.expect("get status").unwrap();
                assert_eq!(status, BlockStatus::Invalid);
            });
        }

        #[test]
        fn proptest_status_transitions_blocking(block in ol_test_utils::ol_block_strategy()) {
            let manager = setup_manager();
            let block_id = block.header().compute_blkid();

            manager.put_block_data_blocking(block.clone()).expect("put block");

            // Set to Valid
            manager.set_block_status_blocking(block_id, BlockStatus::Valid).expect("set valid");
            let status = manager.get_block_status_blocking(block_id).expect("get status").unwrap();
            assert_eq!(status, BlockStatus::Valid);

            // Set to Invalid
            manager.set_block_status_blocking(block_id, BlockStatus::Invalid).expect("set invalid");
            let status = manager.get_block_status_blocking(block_id).expect("get status").unwrap();
            assert_eq!(status, BlockStatus::Invalid);
        }

        #[test]
        fn proptest_get_blocks_at_height_async(mut block1 in ol_test_utils::ol_block_strategy(), mut block2 in ol_test_utils::ol_block_strategy()) {
            let rt = Runtime::new().unwrap();
            rt.block_on(async {
                let manager = setup_manager();
                let slot = 10u64;

                // Override both blocks to same slot
                block1.signed_header.header.slot = slot;
                block2.signed_header.header.slot = slot;

                let block_id1 = block1.header().compute_blkid();
                let block_id2 = block2.header().compute_blkid();

                // Put two blocks at the same slot
                manager.put_block_data_async(block1).await.expect("put block 1");
                manager.put_block_data_async(block2).await.expect("put block 2");

                // Get blocks at height
                let block_ids = manager
                    .get_blocks_at_height_async(slot)
                    .await
                    .expect("get blocks at height");
                assert_eq!(block_ids.len(), 2);
                assert!(block_ids.contains(&block_id1));
                assert!(block_ids.contains(&block_id2));
            });
        }

        #[test]
        fn proptest_canonical_at_returns_indexed_block_not_first(
            mut stale in ol_test_utils::ol_block_strategy(),
            mut canonical in ol_test_utils::ol_block_strategy(),
        ) {
            let slot = 7u64;
            stale.signed_header.header.slot = slot;
            canonical.signed_header.header.slot = slot;
            let stale_id = stale.header().compute_blkid();
            let canonical_id = canonical.header().compute_blkid();
            prop_assume!(stale_id != canonical_id);

            let rt = Runtime::new().unwrap();
            rt.block_on(async {
                let manager = setup_manager();

                // Stale block lands in the height index first, so `.first()` returns it.
                manager.put_block_data_async(stale).await.expect("put stale");
                manager.put_block_data_async(canonical).await.expect("put canonical");
                let at_height = manager
                    .get_blocks_at_height_async(slot)
                    .await
                    .expect("get blocks at height");
                assert_eq!(at_height.first(), Some(&stale_id));

                // Fork choice records the canonical (second) block at the slot.
                manager
                    .replace_canonical_suffix_from_async(slot, vec![canonical_id])
                    .await
                    .expect("seed canonical");

                let got = manager
                    .get_canonical_block_at_async(slot)
                    .await
                    .expect("get canonical")
                    .expect("canonical present");
                assert_eq!(got.blkid(), &canonical_id);
                assert_ne!(got.blkid(), &stale_id);
            });
        }

        #[test]
        fn proptest_get_blocks_at_height_blocking(mut block1 in ol_test_utils::ol_block_strategy(), mut block2 in ol_test_utils::ol_block_strategy()) {
            let manager = setup_manager();
            let slot = 10u64;

            // Override both blocks to same slot
            block1.signed_header.header.slot = slot;
            block2.signed_header.header.slot = slot;

            let block_id1 = block1.header().compute_blkid();
            let block_id2 = block2.header().compute_blkid();

            // Put two blocks at the same slot
            manager.put_block_data_blocking(block1).expect("put block 1");
            manager.put_block_data_blocking(block2).expect("put block 2");

            // Get blocks at height
            let block_ids = manager
                .get_blocks_at_height_blocking(slot)
                .expect("get blocks at height");
            assert_eq!(block_ids.len(), 2);
            assert!(block_ids.contains(&block_id1));
            assert!(block_ids.contains(&block_id2));
        }

        #[test]
        fn proptest_get_tip_slot_async(mut block1 in ol_test_utils::ol_block_strategy(), mut block2 in ol_test_utils::ol_block_strategy()) {
            let rt = Runtime::new().unwrap();
            rt.block_on(async {
                let manager = setup_manager();

                // Override to different slots
                block1.signed_header.header.slot = 5u64;
                block2.signed_header.header.slot = 10u64;

                let block_id1 = block1.header().compute_blkid();
                let block_id2 = block2.header().compute_blkid();

                // Put blocks
                manager.put_block_data_async(block1).await.expect("put block 1");
                manager.put_block_data_async(block2).await.expect("put block 2");

                // Set block2 as valid, but keep the canonical index at block1.
                manager.set_block_status_async(block_id1, BlockStatus::Valid).await.expect("set status");
                manager.set_block_status_async(block_id2, BlockStatus::Valid).await.expect("set status");
                manager
                    .replace_canonical_suffix_from_async(5, vec![block_id1])
                    .await
                    .expect("seed canonical index");

                // Get tip slot - should be 5 (highest canonical slot), not the higher valid fork.
                let tip_slot = manager.get_tip_slot_async().await.expect("get tip slot");
                assert_eq!(tip_slot, 5u64);
            });
        }

        #[test]
        fn proptest_get_tip_slot_blocking(mut block1 in ol_test_utils::ol_block_strategy(), mut block2 in ol_test_utils::ol_block_strategy()) {
            let manager = setup_manager();

            // Override to different slots
            block1.signed_header.header.slot = 5u64;
            block2.signed_header.header.slot = 10u64;

            let block_id1 = block1.header().compute_blkid();
            let block_id2 = block2.header().compute_blkid();

            // Put blocks
            manager.put_block_data_blocking(block1).expect("put block 1");
            manager.put_block_data_blocking(block2).expect("put block 2");

            // Set block2 as valid, but keep the canonical index at block1.
            manager.set_block_status_blocking(block_id1, BlockStatus::Valid).expect("set status");
            manager.set_block_status_blocking(block_id2, BlockStatus::Valid).expect("set status");
            manager
                .replace_canonical_suffix_from_blocking(5, vec![block_id1])
                .expect("seed canonical index");

            // Get tip slot - should be 5 (highest canonical slot), not the higher valid fork.
            let tip_slot = manager.get_tip_slot_blocking().expect("get tip slot");
            assert_eq!(tip_slot, 5u64);
        }
    }

    #[tokio::test]
    async fn test_set_status_nonexistent_async() {
        let manager = setup_manager();
        let nonexistent_id = OLBlockId::from(Buf32::from([0xffu8; 32]));

        let result = manager
            .set_block_status_async(nonexistent_id, BlockStatus::Valid)
            .await;
        assert!(result.is_err());
    }

    #[test]
    fn test_set_status_nonexistent_blocking() {
        let manager = setup_manager();
        let nonexistent_id = OLBlockId::from(Buf32::from([0xffu8; 32]));

        let result = manager.set_block_status_blocking(nonexistent_id, BlockStatus::Valid);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_get_nonexistent_block_async() {
        let manager = setup_manager();
        let nonexistent_id = OLBlockId::from(Buf32::from([0xffu8; 32]));

        let result = manager
            .get_block_data_async(nonexistent_id)
            .await
            .expect("test: get nonexistent");
        assert!(result.is_none());
    }

    #[test]
    fn test_get_nonexistent_block_blocking() {
        let manager = setup_manager();
        let nonexistent_id = OLBlockId::from(Buf32::from([0xffu8; 32]));

        let result = manager
            .get_block_data_blocking(nonexistent_id)
            .expect("test: get nonexistent");
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_delete_nonexistent_block_async() {
        let manager = setup_manager();
        let nonexistent_id = OLBlockId::from(Buf32::from([0xffu8; 32]));

        let existed = manager
            .del_block_data_async(nonexistent_id)
            .await
            .expect("test: delete nonexistent");
        assert!(!existed);
    }

    #[test]
    fn test_delete_nonexistent_block_blocking() {
        let manager = setup_manager();
        let nonexistent_id = OLBlockId::from(Buf32::from([0xffu8; 32]));

        let existed = manager
            .del_block_data_blocking(nonexistent_id)
            .expect("test: delete nonexistent");
        assert!(!existed);
    }
}
