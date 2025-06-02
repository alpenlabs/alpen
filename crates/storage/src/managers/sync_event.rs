//! Sync event db manager.

use std::sync::Arc;

use strata_db::{traits::*, DbResult};
use strata_state::sync_event::SyncEvent;
use threadpool::ThreadPool;

use crate::{cache, ops};

/// Sync event db manager.
#[expect(missing_debug_implementations)]
pub struct SyncEventManager {
    ops: ops::sync_event::SyncEventOps,

    event_cache: cache::CacheTable<u64, Option<SyncEvent>>,
}

impl SyncEventManager {
    pub fn new<D: SyncEventDatabase + Sync + Send + 'static>(pool: ThreadPool, db: Arc<D>) -> Self {
        let ops = ops::sync_event::Context::new(db).into_ops(pool);
        let event_cache = cache::CacheTable::new(64.try_into().unwrap());

        Self { ops, event_cache }
    }

    pub async fn write_sync_event_async(&self, ev: SyncEvent) -> DbResult<u64> {
        let idx = self.ops.write_sync_event_async(ev.clone()).await?;
        self.event_cache.insert(idx, Some(ev));
        Ok(idx)
    }

    pub fn write_sync_event_blocking(&self, ev: SyncEvent) -> DbResult<u64> {
        let idx = self.ops.write_sync_event_blocking(ev.clone())?;
        self.event_cache.insert(idx, Some(ev));
        Ok(idx)
    }

    pub async fn clear_sync_event_range_async(&self, start_idx: u64, end_idx: u64) -> DbResult<()> {
        self.ops
            .clear_sync_event_range_async(start_idx, end_idx)
            .await?;
        self.event_cache
            .purge_if(|k| *k >= start_idx && *k < end_idx);
        Ok(())
    }

    pub fn clear_sync_event_range_blocking(&self, start_idx: u64, end_idx: u64) -> DbResult<()> {
        self.ops
            .clear_sync_event_range_blocking(start_idx, end_idx)?;
        self.event_cache
            .purge_if(|k| *k >= start_idx && *k < end_idx);
        Ok(())
    }

    // TODO convert to keep this cached
    pub async fn get_last_idx_async(&self) -> DbResult<Option<u64>> {
        self.ops.get_last_idx_async().await
    }

    // TODO convert to keep this cached
    pub fn get_last_idx_blocking(&self) -> DbResult<Option<u64>> {
        self.ops.get_last_idx_blocking()
    }

    pub async fn get_sync_event_async(&self, idx: u64) -> DbResult<Option<SyncEvent>> {
        self.event_cache
            .get_or_fetch(&idx, || self.ops.get_sync_event_chan(idx))
            .await
    }

    pub fn get_sync_event_blocking(&self, idx: u64) -> DbResult<Option<SyncEvent>> {
        self.event_cache
            .get_or_fetch_blocking(&idx, || self.ops.get_sync_event_blocking(idx))
    }
}
