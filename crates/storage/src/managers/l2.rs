use std::sync::Arc;

use strata_db::{
    traits::{BlockStatus, L2BlockDatabase},
    DbResult,
};
use strata_ol_chain_types::{L2BlockBundle, L2BlockId, L2Header};
use threadpool::ThreadPool;

use crate::{cache, ops};

/// Caching manager of L2 blocks in the block database.
#[expect(
    missing_debug_implementations,
    reason = "Some inner types don't have Debug implementation"
)]
pub struct L2BlockManager {
    ops: ops::l2::L2DataOps,
    block_cache: cache::CacheTable<L2BlockId, Option<L2BlockBundle>>,
}

impl L2BlockManager {
    pub fn new(pool: ThreadPool, db: Arc<impl L2BlockDatabase + 'static>) -> Self {
        let ops = ops::l2::Context::new(db).into_ops(pool);
        let block_cache = cache::CacheTable::new(64.try_into().unwrap());
        Self { ops, block_cache }
    }

    /// Puts a block in the database, purging cache entry.
    pub async fn put_block_data_async(&self, bundle: L2BlockBundle) -> DbResult<()> {
        let header = bundle.block().header().clone();
        let id = header.get_blockid();
        self.ops.put_block_data_async(bundle).await?;
        self.block_cache.purge_async(&id).await;
        Ok(())
    }

    /// Puts in a block in the database, purging cache entry.
    pub fn put_block_data_blocking(&self, bundle: L2BlockBundle) -> DbResult<()> {
        let header = bundle.block().header().clone();
        let id = header.get_blockid();
        self.ops.put_block_data_blocking(bundle)?;
        self.block_cache.purge_blocking(&id);
        Ok(())
    }

    /// Gets a block either in the cache or from the underlying database.
    pub async fn get_block_data_async(&self, id: &L2BlockId) -> DbResult<Option<L2BlockBundle>> {
        self.block_cache
            .get_or_fetch(id, || self.ops.get_block_data_chan(*id))
            .await
    }

    /// Gets a block either in the cache or from the underlying database.
    pub fn get_block_data_blocking(&self, id: &L2BlockId) -> DbResult<Option<L2BlockBundle>> {
        self.block_cache
            .get_or_fetch_blocking(id, || self.ops.get_block_data_blocking(*id))
    }

    /// Gets the block at a height.  Async.
    pub async fn get_blocks_at_height_async(&self, h: u64) -> DbResult<Vec<L2BlockId>> {
        self.ops.get_blocks_at_height_async(h).await
    }

    /// Gets the block at a height.  Blocking.
    pub fn get_blocks_at_height_blocking(&self, h: u64) -> DbResult<Vec<L2BlockId>> {
        self.ops.get_blocks_at_height_blocking(h)
    }

    /// Gets the block at a height.  Async.
    pub async fn get_tip_block_async(&self) -> DbResult<L2BlockId> {
        self.ops.get_tip_block_async().await
    }

    /// Gets the block at a height.  Blocking.
    pub fn get_tip_block_blocking(&self) -> DbResult<L2BlockId> {
        self.ops.get_tip_block_blocking()
    }

    /// Gets the block's verification status.  Async.
    pub async fn get_block_status_async(&self, id: &L2BlockId) -> DbResult<Option<BlockStatus>> {
        self.ops.get_block_status_async(*id).await
    }

    /// Gets the block's verification status.  Blocking.
    pub fn get_block_status_blocking(&self, id: &L2BlockId) -> DbResult<Option<BlockStatus>> {
        self.ops.get_block_status_blocking(*id)
    }

    /// Sets the block's verification status.  Async.
    pub async fn set_block_status_async(
        &self,
        id: &L2BlockId,
        status: BlockStatus,
    ) -> DbResult<()> {
        self.ops.set_block_status_async(*id, status).await?;

        Ok(())
    }

    /// Sets the block's verification status.  Blocking.
    pub fn set_block_status_blocking(&self, id: &L2BlockId, status: BlockStatus) -> DbResult<()> {
        self.ops.set_block_status_blocking(*id, status)?;

        Ok(())
    }
}
