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
use tokio::time::{sleep, Duration};
use tracing::info;

use crate::errors::CheckpointSyncError;
type SyncResult<T> = std::result::Result<T, CheckpointSyncError>;

#[async_trait]
pub trait CheckpointSync {
    /// Wait until an epoch (i.e. corresponding checkpoint transaction) is finalized
    async fn wait_for_epoch_finalization(&self) -> SyncResult<Option<u64>>;

    /// Extract chainstate diff from checkpoint.
    fn get_chainstate_diff_from_checkpoint(
        &self,
        checkpoint: &Checkpoint,
    ) -> SyncResult<impl ChainstateDiff>;

    /// Get the next epoch (target) which needs to be finalized.
    async fn get_target_epoch(&self) -> SyncResult<u64>;

    /// Get checkpoint by epoch number (checkpoints are indexed by epoch number).
    async fn get_checkpoint_by_epoch(&self, epoch: u64) -> SyncResult<Checkpoint>;

    /// Get latest chainstate maintained by the client (which is chainstate for the latest slot).
    async fn get_latest_chainstate(&self) -> SyncResult<Chainstate>;

    /// Store chainstate.
    async fn store_chainstate(&mut self, chainstate: Chainstate) -> SyncResult<()>;

    /// Send status update to listeners of checkpoint sync service.
    fn notify_sync_status_update(&self, update: ChainSyncStatusUpdate);
}

pub struct CheckpointSyncManager {
    storage: Arc<NodeStorage>,
    status_channel: StatusChannel,
}

impl CheckpointSyncManager {
    pub fn new(storage: Arc<NodeStorage>, status_channel: StatusChannel) -> Self {
        Self {
            storage,
            status_channel,
        }
    }
}

#[async_trait]
impl CheckpointSync for CheckpointSyncManager {
    async fn wait_for_epoch_finalization(&self) -> SyncResult<Option<u64>> {
        let mut rx = self.status_channel.subscribe_client_state();
        rx.changed().await?;
        let finalized_epoch = rx
            .borrow()
            .get_apparent_finalized_checkpoint()
            .map(|c| c.batch_info.epoch);
        Ok(finalized_epoch)
    }

    async fn get_target_epoch(&self) -> SyncResult<u64> {
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

    async fn get_latest_chainstate(&self) -> SyncResult<Chainstate> {
        let chsman = self.storage.chainstate();
        let idx = chsman.get_last_write_idx_async().await?;
        let latest_chainstate = chsman
            .get_toplevel_chainstate_async(idx)
            .await?
            .ok_or(DbError::MissingL2State(idx))?
            .to_chainstate();
        Ok(latest_chainstate)
    }

    async fn store_chainstate(&mut self, chainstate: Chainstate) -> SyncResult<()> {
        let chsman = self.storage.chainstate();
        let slot = chainstate.chain_tip_slot();
        let block_commitment = chainstate.finalized_epoch().to_block_commitment();
        let write_batch = WriteBatchEntry::new(
            WriteBatch::new(chainstate, Vec::new()),
            block_commitment.blkid().to_owned(),
        );
        chsman.put_write_batch_async(slot, write_batch).await?;
        Ok(())
    }

    async fn get_checkpoint_by_epoch(&self, epoch: u64) -> SyncResult<Checkpoint> {
        if let Some(entry) = self.storage.checkpoint().get_checkpoint(epoch).await? {
            Ok(entry.into_batch_checkpoint())
        } else {
            Err(CheckpointSyncError::MissingCheckpoint(epoch).into())
        }
    }

    fn get_chainstate_diff_from_checkpoint(
        &self,
        checkpoint: &Checkpoint,
    ) -> SyncResult<impl ChainstateDiff> {
        match FullStateUpdate::from_buf(checkpoint.sidecar().bytes()) {
            Ok(state_diff) => Ok(state_diff),
            Err(err) => Err(CheckpointSyncError::FailedDiffExtraction(format!("{}", err)).into()),
        }
    }

    fn notify_sync_status_update(&self, update: ChainSyncStatusUpdate) {
        self.status_channel.update_chain_sync_status(update);
    }
}

pub async fn checkpoint_sync_task(mut csync_manager: impl CheckpointSync) -> anyhow::Result<()> {
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
        } else {
            // avoid tight loops if no epoch is finalized yet
            sleep(Duration::from_secs(1)).await;
        }
    }
}

/// Process checkpoints for epochs that have been finalized.
async fn sync_checkpoint_for_finalized_epochs(
    target_epoch: u64,
    csm_finalized_epoch: u64,
    csync_manager: &mut impl CheckpointSync,
) -> SyncResult<u64> {
    let current_epoch = target_epoch.saturating_sub(1);
    if csm_finalized_epoch < current_epoch {
        unimplemented!("reorg beyond finalization depth is undefined for strata client!");
    }

    let mut next_target_epoch = target_epoch;
    for epoch in target_epoch..=csm_finalized_epoch {
        let checkpoint = csync_manager.get_checkpoint_by_epoch(epoch).await?;
        if let Err(err) = process_checkpoint(&checkpoint, csync_manager).await {
            tracing::warn!(%epoch, %err, "Failed to process malformed checkpoint; returning early");
            return Ok(next_target_epoch);
        }
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
    csync_manager: &mut impl CheckpointSync,
) -> SyncResult<()> {
    // apply diff to chainstate
    let mut chainstate = csync_manager.get_latest_chainstate().await?;
    csync_manager
        .get_chainstate_diff_from_checkpoint(checkpoint)?
        .apply_to_chainstate(&mut chainstate)
        .map_err(|err| CheckpointSyncError::FailedDiffApplication(format!("{:?}", err)))?;

    // store updated chainstate
    let slot = chainstate.chain_tip_slot();
    let blkid = checkpoint.batch_info().final_l2_block().blkid();
    let epoch = checkpoint.batch_info().epoch();
    info!(%epoch, %slot, %blkid, "storing updated chainstate");
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
