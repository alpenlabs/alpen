use std::sync::Arc;

use async_trait::async_trait;
#[cfg(feature = "debug-utils")]
use strata_common::{check_and_pause_debug_async, WorkerType};
use strata_db::DbError;
use strata_state::{
    batch::Checkpoint,
    chain_state::{Chainstate, FullStateUpdate},
    state_op::{WriteBatch, WriteBatchEntry},
    traits::ChainstateDiff,
};
use strata_status::{ChainSyncStatus, ChainSyncStatusUpdate, StatusChannel};
use strata_storage::NodeStorage;

use crate::errors::CheckpointSyncError;

#[async_trait]
pub trait CheckpointSyncManager {
    /// Wait until an epoch (i.e. corresponding checkpoint transaction) is finalized
    async fn wait_for_epoch_finalization(&self) -> anyhow::Result<Option<u64>>;

    /// Extract chainstate diff from checkpoint.
    fn get_chainstate_diff_from_checkpoint(
        &self,
        checkpoint: &Checkpoint,
    ) -> anyhow::Result<impl ChainstateDiff>;

    /// Get the next epoch (target) which needs to be finalized.
    async fn get_target_epoch(&self) -> anyhow::Result<u64>;

    /// Get checkpoint by epoch number (checkpoints are indexed by epoch number).
    async fn get_checkpoint_by_epoch(&self, epoch: u64) -> anyhow::Result<Checkpoint>;

    /// Get latest chainstate maintained by the client (which is chainstate for the latest slot).
    async fn get_latest_chainstate(&self) -> anyhow::Result<Chainstate>;

    /// Store chainstate.
    async fn store_chainstate(&mut self, chainstate: Chainstate) -> anyhow::Result<()>;

    /// Send status update to listeners of checkpoint sync service.
    fn notify_sync_status_update(&self, update: ChainSyncStatusUpdate);
}

pub struct CheckpointSyncManagerImpl {
    storage: Arc<NodeStorage>,
    status_channel: StatusChannel,
}

impl CheckpointSyncManagerImpl {
    pub fn new(storage: Arc<NodeStorage>, status_channel: StatusChannel) -> Self {
        Self {
            storage,
            status_channel,
        }
    }
}

#[async_trait]
impl CheckpointSyncManager for CheckpointSyncManagerImpl {
    async fn wait_for_epoch_finalization(&self) -> anyhow::Result<Option<u64>> {
        let mut rx = self.status_channel.subscribe_client_state();
        rx.changed().await?;

        /// TODO: could be a better way to get finalized epoch rather than from checkpoint
        let finalized_epoch = rx
            .borrow()
            .get_apparent_finalized_checkpoint()
            .map(|c| c.batch_info.epoch);
        Ok(finalized_epoch)
    }

    async fn get_target_epoch(&self) -> anyhow::Result<u64> {
        let chsman = self.storage.chainstate();
        let Some(slot) = chsman.get_last_write_idx_async().await.ok() else {
            return Ok(0);
        };

        match chsman.get_toplevel_chainstate_async(slot).await {
            Ok(Some(entry)) => Ok(entry.state().cur_epoch() + 1),
            Ok(None) | Err(DbError::NotBootstrapped) => Ok(0),
            Err(err) => Err(err.into()),
        }
    }

    async fn get_latest_chainstate(&self) -> anyhow::Result<Chainstate> {
        let chsman = self.storage.chainstate();
        let idx = chsman.get_last_write_idx_async().await?;
        let latest_chainstate = chsman
            .get_toplevel_chainstate_async(idx)
            .await?
            .ok_or(DbError::MissingL2State(idx))?
            .to_chainstate();
        Ok(latest_chainstate)
    }

    async fn store_chainstate(&mut self, chainstate: Chainstate) -> anyhow::Result<()> {
        let chsman = self.storage.chainstate();
        let block_commitment = chainstate.finalized_epoch().to_block_commitment();
        let wb = WriteBatchEntry::new(
            WriteBatch::new(chainstate.clone(), Vec::new()),
            block_commitment.blkid().to_owned(),
        );
        chsman
            .put_write_batch_async(chainstate.chain_tip_slot(), wb)
            .await?;
        Ok(())
    }

    async fn get_checkpoint_by_epoch(&self, epoch: u64) -> anyhow::Result<Checkpoint> {
        if let Some(entry) = self.storage.checkpoint().get_checkpoint(epoch).await? {
            Ok(entry.into_batch_checkpoint())
        } else {
            Err(CheckpointSyncError::MissingCheckpoint(epoch).into())
        }
    }

    fn get_chainstate_diff_from_checkpoint(
        &self,
        checkpoint: &Checkpoint,
    ) -> anyhow::Result<impl ChainstateDiff> {
        Ok(FullStateUpdate::from_buf(checkpoint.sidecar().bytes())?)
    }

    fn notify_sync_status_update(&self, update: ChainSyncStatusUpdate) {
        self.status_channel.update_chain_sync_status(update);
    }
}

pub async fn checkpoint_sync_task_v2(
    mut csync_manager: impl CheckpointSyncManager,
) -> anyhow::Result<()> {
    let mut target_epoch = csync_manager.get_target_epoch().await?;

    loop {
        #[cfg(feature = "debug-utils")]
        check_and_pause_debug_async(WorkerType::CheckpointSyncWorker).await;

        if let Some(csm_finalized_epoch) = csync_manager.wait_for_epoch_finalization().await? {
            target_epoch = sync_checkpoint_for_finalized_epochs(
                target_epoch,
                csm_finalized_epoch,
                &mut csync_manager,
            )
            .await?;
        }
    }
}

/// Process checkpoints for epochs that have been finalized.
async fn sync_checkpoint_for_finalized_epochs(
    target_epoch: u64,
    csm_finalized_epoch: u64,
    csync_manager: &mut impl CheckpointSyncManager,
) -> anyhow::Result<u64> {
    // this is a reorg condition
    let current_epoch = target_epoch.saturating_sub(1);
    if csm_finalized_epoch < current_epoch {
        unimplemented!("reorg beyond finalization depth is undefined for strata client!");
    }

    let mut next_target_epoch = target_epoch;
    for epoch in target_epoch..=csm_finalized_epoch {
        let checkpoint = csync_manager.get_checkpoint_by_epoch(epoch).await?;
        process_checkpoint(&checkpoint, csync_manager).await?;
        next_target_epoch += 1;
    }
    Ok(next_target_epoch)
}

/// Process a single checkpoint.
/// 1. Extract chainstate diff from checkpoint.
/// 2. Apply diff to chainstate available from last processed checkpoint (which is the latest
///    chainstate in this case).
/// 3. Store updated chainstate to database.
/// 4. Send chainstate update information to other components.
async fn process_checkpoint(
    checkpoint: &Checkpoint,
    csync_manager: &mut impl CheckpointSyncManager,
) -> anyhow::Result<()> {
    let mut chainstate = csync_manager.get_latest_chainstate().await?;

    // apply diff and store chainstate
    csync_manager
        .get_chainstate_diff_from_checkpoint(checkpoint)?
        .apply_to_chainstate(&mut chainstate)?;

    csync_manager.store_chainstate(chainstate.clone()).await?;

    // send status update
    let tip = checkpoint.batch_info().final_l2_block();
    let status = ChainSyncStatus::new(
        *tip,
        *chainstate.prev_epoch(),
        *chainstate.finalized_epoch(),
        chainstate.l1_view().get_safe_block(),
    );
    let update = ChainSyncStatusUpdate::new(status, Arc::new(chainstate));
    csync_manager.notify_sync_status_update(update);

    Ok(())
}
