//! High-level chainstate interface.

use std::sync::Arc;

use strata_db::{traits::*, DbResult};
use strata_state::{
    chain_state::{Chainstate, ChainstateEntry},
    id::L2BlockId,
    state_op::WriteBatchEntry,
};
use threadpool::ThreadPool;

use crate::{cache, ops};

#[expect(missing_debug_implementations)]
pub struct ChainstateManager {
    ops: ops::chainstate::ChainstateOps,
    wb_cache: cache::CacheTable<u64, Option<WriteBatchEntry>>,
}

impl ChainstateManager {
    pub fn new<D: Database + Sync + Send + 'static>(pool: ThreadPool, db: Arc<D>) -> Self {
        let ops = ops::chainstate::Context::new(db.chain_state_db().clone()).into_ops(pool);
        let wb_cache = cache::CacheTable::new(64.try_into().unwrap());
        Self { ops, wb_cache }
    }

    // Basic functions that map directly onto database operations.

    /// Writes the genesis state.  This only exists in blocking form because
    /// that's all we need.
    pub fn write_genesis_state(
        &self,
        toplevel: Chainstate,
        genesis_blkid: L2BlockId,
    ) -> DbResult<()> {
        self.ops
            .write_genesis_state_blocking(toplevel, genesis_blkid)
    }

    /// Stores a new write batch at a particular index.
    pub async fn put_write_batch_async(&self, idx: u64, wb: WriteBatchEntry) -> DbResult<()> {
        self.ops.put_write_batch_async(idx, wb).await?;
        self.wb_cache.purge(&idx);
        Ok(())
    }

    /// Stores a new write batch at a particular index.
    pub fn put_write_batch_blocking(&self, idx: u64, wb: WriteBatchEntry) -> DbResult<()> {
        self.ops.put_write_batch_blocking(idx, wb)?;
        self.wb_cache.purge(&idx);
        Ok(())
    }

    /// Gets the writes stored for an index.
    pub async fn get_write_batch_async(&self, idx: u64) -> DbResult<Option<WriteBatchEntry>> {
        self.wb_cache
            .get_or_fetch(&idx, || self.ops.get_write_batch_chan(idx))
            .await
    }

    /// Gets the writes stored for an index.
    pub fn get_write_batch_blocking(&self, idx: u64) -> DbResult<Option<WriteBatchEntry>> {
        self.wb_cache
            .get_or_fetch_blocking(&idx, || self.ops.get_write_batch_blocking(idx))
    }

    pub async fn purge_entries_before_async(&self, before_idx: u64) -> DbResult<()> {
        self.ops.purge_entries_before_async(before_idx).await?;
        self.wb_cache.purge_if(|k| *k < before_idx);
        Ok(())
    }

    pub fn purge_entries_before_blocking(&self, before_idx: u64) -> DbResult<()> {
        self.ops.purge_entries_before_blocking(before_idx)?;
        self.wb_cache.purge_if(|k| *k < before_idx);
        Ok(())
    }

    /// Rolls back writes after a given new tip index, making it the newest tip.
    pub async fn rollback_writes_to_async(&self, new_tip_idx: u64) -> DbResult<()> {
        self.ops.rollback_writes_to_async(new_tip_idx).await?;
        self.wb_cache.purge_if(|k| *k > new_tip_idx);
        Ok(())
    }

    /// Rolls back writes after a given new tip index, making it the newest tip.
    pub fn rollback_writes_to_blocking(&self, new_tip_idx: u64) -> DbResult<()> {
        self.ops.rollback_writes_to_blocking(new_tip_idx)?;
        self.wb_cache.purge_if(|k| *k > new_tip_idx);
        Ok(())
    }

    pub async fn get_earliest_write_idx_async(&self) -> DbResult<u64> {
        // TODO convert to keep this cached in memory so we don't need both variants
        self.ops.get_earliest_write_idx_async().await
    }

    pub fn get_earliest_write_idx_blocking(&self) -> DbResult<u64> {
        // TODO convert to keep this cached in memory so we don't need both variants
        self.ops.get_earliest_write_idx_blocking()
    }

    pub async fn get_last_write_idx_async(&self) -> DbResult<u64> {
        // TODO convert to keep this cached in memory so we don't need both variants
        self.ops.get_last_write_idx_async().await
    }

    pub fn get_last_write_idx_blocking(&self) -> DbResult<u64> {
        // TODO convert to keep this cached in memory so we don't need both variants
        self.ops.get_last_write_idx_blocking()
    }

    // Nontrivial functions that aren't just 1:1.

    /// Convenience function just for extracting the toplevel chainstate from
    /// the write batch at an index.
    pub async fn get_toplevel_chainstate_async(
        &self,
        idx: u64,
    ) -> DbResult<Option<ChainstateEntry>> {
        Ok(self.get_write_batch_async(idx).await?.map(Into::into))
    }

    /// Convenience function just for extracting the toplevel chainstate from
    /// the write batch at an index.
    pub fn get_toplevel_chainstate_blocking(&self, idx: u64) -> DbResult<Option<ChainstateEntry>> {
        Ok(self.get_write_batch_blocking(idx)?.map(Into::into))
    }
}
