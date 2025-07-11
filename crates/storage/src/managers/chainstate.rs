//! High-level new chainstate interface.

use std::sync::Arc;

use futures::TryFutureExt;
use strata_db::{
    chainstate::{ChainstateDatabase, StateInstanceId, WriteBatchId},
    DbResult,
};
use strata_primitives::buf::Buf32;
use strata_state::{chain_state::Chainstate, id::L2BlockId, state_op::WriteBatch};
use threadpool::ThreadPool;
use tracing::*;

use crate::{cache, ops};

#[expect(missing_debug_implementations)]
pub struct ChainstateManager {
    ops: ops::chainstate::ChainstateOps,
    tl_cache: cache::CacheTable<StateInstanceId, Arc<Chainstate>>,
    wb_cache: cache::CacheTable<WriteBatchId, Option<WriteBatch>>,
}

impl ChainstateManager {
    pub fn new<D: ChainstateDatabase + Sync + Send + 'static>(
        pool: ThreadPool,
        db: Arc<D>,
    ) -> Self {
        let ops = ops::chainstate::Context::new(db.clone()).into_ops(pool);
        let tl_cache = cache::CacheTable::new(64.try_into().unwrap());
        let wb_cache = cache::CacheTable::new(64.try_into().unwrap());
        Self {
            ops,
            tl_cache,
            wb_cache,
        }
    }

    /// Creates a new state instance.
    pub async fn create_new_inst_async(self, toplevel: Chainstate) -> DbResult<StateInstanceId> {
        let id = self.ops.create_new_inst_async(toplevel.clone()).await?;
        self.tl_cache.insert(id, Arc::new(toplevel));
        Ok(id)
    }

    /// Creates a new state instance.
    pub fn create_new_inst_blocking(&self, toplevel: Chainstate) -> DbResult<StateInstanceId> {
        let id = self.ops.create_new_inst_blocking(toplevel.clone())?;
        self.tl_cache.insert(id, Arc::new(toplevel));
        Ok(id)
    }

    /// Clones an existing state instance.
    pub async fn clone_inst_async(&self, id: StateInstanceId) -> DbResult<StateInstanceId> {
        self.ops.clone_inst_async(id).await
    }

    /// Clones an existing state instance.
    pub fn clone_inst_blocking(&self, id: StateInstanceId) -> DbResult<StateInstanceId> {
        self.ops.clone_inst_blocking(id)
    }

    /// Deletes a state instance.
    pub async fn del_inst_async(&self, id: StateInstanceId) -> DbResult<()> {
        self.ops.del_inst_async(id).await?;
        self.tl_cache.purge(&id);
        Ok(())
    }

    /// Deletes a state instance.
    pub fn del_inst_blocking(&self, id: StateInstanceId) -> DbResult<()> {
        self.ops.del_inst_blocking(id)?;
        self.tl_cache.purge(&id);
        Ok(())
    }

    /// Gets the list of state instances.
    pub async fn get_insts_async(&self) -> DbResult<Vec<StateInstanceId>> {
        self.ops.get_insts_async().await
    }

    /// Gets the list of state instances.
    pub fn get_insts_blocking(&self) -> DbResult<Vec<StateInstanceId>> {
        self.ops.get_insts_blocking()
    }

    /// Gets the state instance's toplevel state.
    ///
    /// Note: This currently ignores the toplevel chainstate cache.
    pub async fn get_inst_toplevel_state_async(
        &self,
        id: StateInstanceId,
    ) -> DbResult<Arc<Chainstate>> {
        // TODO this is slow, but we need to do it, we need to do it because we
        // didn't have async fns until recently
        warn!("fetching instance toplevel state via async fn, bypassing cache due to limitations");
        self.ops
            .get_inst_toplevel_state_async(id)
            .map_ok(Arc::new)
            .await
    }

    /// Gets the state instance's toplevel state.
    pub fn get_inst_toplevel_state_blocking(
        &self,
        id: StateInstanceId,
    ) -> DbResult<Arc<Chainstate>> {
        self.tl_cache.get_or_fetch_blocking(&id, || {
            self.ops.get_inst_toplevel_state_blocking(id).map(Arc::new)
        })
    }

    /// Puts a new write batch with some ID.
    pub async fn put_write_batch_async(&self, id: WriteBatchId, wb: WriteBatch) -> DbResult<()> {
        self.ops.put_write_batch_async(id, wb.clone()).await?;
        self.wb_cache.insert(id, Some(wb));
        Ok(())
    }

    /// Puts a new write batch with some ID.
    pub fn put_write_batch_blocking(&self, id: WriteBatchId, wb: WriteBatch) -> DbResult<()> {
        self.ops.put_write_batch_blocking(id, wb.clone())?;
        self.wb_cache.insert(id, Some(wb));
        Ok(())
    }

    /// Puts a new write batch for a slot
    pub async fn put_slot_write_batch_async(&self, id: L2BlockId, wb: WriteBatch) -> DbResult<()> {
        let wb_id = conv_blkid_to_slot_wb_id(id);
        self.put_write_batch_async(wb_id, wb).await
    }

    /// Puts a new write batch for an epoch
    pub async fn put_epoch_write_batch_async(&self, id: L2BlockId, wb: WriteBatch) -> DbResult<()> {
        let wb_id = conv_blkid_to_epoch_terminal_wb_id(id);
        self.put_write_batch_async(wb_id, wb).await
    }

    /// Puts a new write batch for a slot
    pub fn put_slot_write_batch_blocking(&self, id: L2BlockId, wb: WriteBatch) -> DbResult<()> {
        let wb_id = conv_blkid_to_slot_wb_id(id);
        self.put_write_batch_blocking(wb_id, wb)
    }

    /// Puts a new write batch for an epoch
    pub fn put_epoch_write_batch_blocking(&self, id: L2BlockId, wb: WriteBatch) -> DbResult<()> {
        let wb_id = conv_blkid_to_epoch_terminal_wb_id(id);
        self.put_write_batch_blocking(wb_id, wb)
    }

    /// Gets a write batch with some ID.
    pub async fn get_write_batch_async(&self, id: WriteBatchId) -> DbResult<Option<WriteBatch>> {
        self.wb_cache
            .get_or_fetch(&id, || self.ops.get_write_batch_chan(id))
            .await
    }

    /// Gets a write batch with some ID.
    pub fn get_write_batch_blocking(&self, id: WriteBatchId) -> DbResult<Option<WriteBatch>> {
        self.wb_cache
            .get_or_fetch_blocking(&id, || self.ops.get_write_batch_blocking(id))
    }

    /// Gets a write batch with some ID.
    pub async fn get_slot_write_batch_async(&self, id: L2BlockId) -> DbResult<Option<WriteBatch>> {
        let wb_id = conv_blkid_to_slot_wb_id(id);
        self.get_write_batch_async(wb_id).await
    }

    /// Gets a write batch with some ID.
    pub fn get_slot_write_batch_blocking(&self, id: L2BlockId) -> DbResult<Option<WriteBatch>> {
        let wb_id = conv_blkid_to_slot_wb_id(id);
        self.get_write_batch_blocking(wb_id)
    }

    /// Gets a write batch with some ID.
    pub async fn get_epoch_write_batch_async(&self, id: L2BlockId) -> DbResult<Option<WriteBatch>> {
        let wb_id = conv_blkid_to_epoch_terminal_wb_id(id);
        self.get_write_batch_async(wb_id).await
    }

    /// Gets a write batch with some ID.
    pub fn get_epoch_write_batch_blocking(&self, id: L2BlockId) -> DbResult<Option<WriteBatch>> {
        let wb_id = conv_blkid_to_epoch_terminal_wb_id(id);
        self.get_write_batch_blocking(wb_id)
    }

    /// Deletes a write batch with some ID.
    pub async fn del_write_batch_async(&self, id: WriteBatchId) -> DbResult<()> {
        self.ops.del_write_batch_async(id).await?;
        self.wb_cache.purge(&id);
        Ok(())
    }

    /// Deletes a write batch with some ID.
    pub fn del_write_batch_blocking(&self, id: WriteBatchId) -> DbResult<()> {
        self.ops.del_write_batch_blocking(id)?;
        self.wb_cache.purge(&id);
        Ok(())
    }

    /// Merges a list of changes into a write batch.
    pub async fn merge_write_batches(
        &self,
        id: StateInstanceId,
        wb_ids: Vec<WriteBatchId>,
    ) -> DbResult<()> {
        self.ops.merge_write_batches_async(id, wb_ids).await?;

        // FIXME this is inefficient, but it's safer than potentially leaving
        // stale or messed-up data in the cache, we should have some more
        // general function for preparing a cache slot and waiting on a fn call
        // to fill it
        self.tl_cache.purge(&id);

        Ok(())
    }

    /// Merges a list of changes into a write batch.
    pub fn merge_write_batches_blocking(
        &self,
        id: StateInstanceId,
        wb_ids: Vec<WriteBatchId>,
    ) -> DbResult<()> {
        self.ops.merge_write_batches_blocking(id, wb_ids)?;

        // FIXME see above
        self.tl_cache.purge(&id);

        Ok(())
    }
}

fn conv_blkid_to_slot_wb_id(blkid: L2BlockId) -> WriteBatchId {
    let mut buf: Buf32 = blkid.into();
    buf.as_mut_slice()[31] = 0; // last byte to distinguish slot and epoch
    buf
}

fn conv_blkid_to_epoch_terminal_wb_id(blkid: L2BlockId) -> WriteBatchId {
    let mut buf: Buf32 = blkid.into();
    buf.as_mut_slice()[31] = 1; // last byte to distinguish slot and epoch
    buf
}
