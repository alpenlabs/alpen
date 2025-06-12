use std::sync::Arc;

use strata_db::{traits::CheckpointDatabase, types::CheckpointEntry, DbResult};
use strata_primitives::epoch::EpochCommitment;
use strata_state::batch::EpochSummary;
use threadpool::ThreadPool;

use crate::{cache, ops};

#[expect(missing_debug_implementations)]
pub struct CheckpointDbManager {
    ops: ops::checkpoint::CheckpointDataOps,
    summary_cache: cache::CacheTable<EpochCommitment, Option<EpochSummary>>,
    checkpoint_cache: cache::CacheTable<u64, Option<CheckpointEntry>>,
}

impl CheckpointDbManager {
    pub fn new<D: CheckpointDatabase + Sync + Send + 'static>(
        pool: ThreadPool,
        db: Arc<D>,
    ) -> Self {
        let ops = ops::checkpoint::Context::new(db).into_ops(pool);
        let summary_cache = cache::CacheTable::new(64.try_into().unwrap());
        let checkpoint_cache = cache::CacheTable::new(64.try_into().unwrap());
        Self {
            ops,
            summary_cache,
            checkpoint_cache,
        }
    }

    pub async fn insert_epoch_summary(&self, summary: EpochSummary) -> DbResult<()> {
        self.ops.insert_epoch_summary_async(summary).await?;
        self.summary_cache
            .insert(summary.get_epoch_commitment(), Some(summary));
        Ok(())
    }

    pub fn insert_epoch_summary_blocking(&self, summary: EpochSummary) -> DbResult<()> {
        self.ops.insert_epoch_summary_blocking(summary)?;
        self.summary_cache
            .insert(summary.get_epoch_commitment(), Some(summary));
        Ok(())
    }

    pub async fn get_epoch_summary(
        &self,
        epoch: EpochCommitment,
    ) -> DbResult<Option<EpochSummary>> {
        self.summary_cache
            .get_or_fetch(&epoch, || self.ops.get_epoch_summary_chan(epoch))
            .await
    }

    pub fn get_epoch_summary_blocking(
        &self,
        epoch: EpochCommitment,
    ) -> DbResult<Option<EpochSummary>> {
        self.summary_cache
            .get_or_fetch_blocking(&epoch, || self.ops.get_epoch_summary_blocking(epoch))
    }

    pub async fn get_last_summarized_epoch(&self) -> DbResult<Option<u64>> {
        // TODO cache this?
        self.ops.get_last_summarized_epoch_async().await
    }

    pub fn get_last_summarized_epoch_blocking(&self) -> DbResult<Option<u64>> {
        // TODO cache this?
        self.ops.get_last_summarized_epoch_blocking()
    }

    /// Gets the epoch commitments for some epoch.
    ///
    /// Note that this bypasses the epoch summary cache, so always may cause a
    /// disk fetch even if called repeatedly.
    pub async fn get_epoch_commitments_at(&self, epoch: u64) -> DbResult<Vec<EpochCommitment>> {
        self.ops.get_epoch_commitments_at_async(epoch).await
    }

    /// Note that this bypasses the epoch summary cache.
    ///
    /// Note that this bypasses the epoch summary cache, so always may cause a
    /// disk fetch even if called repeatedly.
    pub fn get_epoch_commitments_at_blocking(&self, epoch: u64) -> DbResult<Vec<EpochCommitment>> {
        self.ops.get_epoch_commitments_at_blocking(epoch)
    }

    pub async fn put_checkpoint(&self, idx: u64, entry: CheckpointEntry) -> DbResult<()> {
        self.ops.put_checkpoint_async(idx, entry).await?;
        self.checkpoint_cache.purge(&idx);
        Ok(())
    }

    pub fn put_checkpoint_blocking(&self, idx: u64, entry: CheckpointEntry) -> DbResult<()> {
        self.ops.put_checkpoint_blocking(idx, entry)?;
        self.checkpoint_cache.purge(&idx);
        Ok(())
    }

    pub async fn get_checkpoint(&self, idx: u64) -> DbResult<Option<CheckpointEntry>> {
        self.checkpoint_cache
            .get_or_fetch(&idx, || self.ops.get_checkpoint_chan(idx))
            .await
    }

    pub fn get_checkpoint_blocking(&self, idx: u64) -> DbResult<Option<CheckpointEntry>> {
        self.checkpoint_cache
            .get_or_fetch_blocking(&idx, || self.ops.get_checkpoint_blocking(idx))
    }

    pub async fn get_last_checkpoint(&self) -> DbResult<Option<u64>> {
        self.ops.get_last_checkpoint_idx_async().await
    }

    pub fn get_last_checkpoint_blocking(&self) -> DbResult<Option<u64>> {
        self.ops.get_last_checkpoint_idx_blocking()
    }
}
