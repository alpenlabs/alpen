use std::sync::Arc;

#[cfg(feature = "debug-utils")]
use strata_common::{check_and_pause_debug_async, WorkerType};
use strata_db::{types::CheckpointEntry, DbError};
use strata_primitives::l2::L2BlockCommitment;
use strata_state::{
    batch::Checkpoint,
    chain_state::{Chainstate, FullStateUpdate},
    client_state::L1Checkpoint,
    state_op::{WriteBatch, WriteBatchEntry},
    traits::ChainstateDiff,
};
use strata_status::{ChainSyncStatus, ChainSyncStatusUpdate, StatusChannel};
use strata_storage::NodeStorage;
use tracing::{error, info, warn};

// should we maintain an in-memory structure for this?
struct ChainstateStorageInterface {
    /// Common node storage interface.
    storage: Arc<NodeStorage>,
}

impl ChainstateStorageInterface {
    pub(crate) fn new(storage: Arc<NodeStorage>) -> Self {
        Self { storage }
    }

    /// Apply chainstate diff to latest chainstate.
    pub(crate) async fn apply_diff_to_latest_chainstate(
        &mut self,
        chainstate_diff: impl ChainstateDiff,
    ) -> anyhow::Result<Chainstate> {
        // no chainstate in db means genesis chainstate hasn't been initialized yet
        // raise not bootstrapped error in that case
        let mut latest_chainstate = self
            .get_latest_chainstate()
            .await?
            .ok_or(DbError::NotBootstrapped)?;
        chainstate_diff.apply_to_chainstate(&mut latest_chainstate)?;
        Ok(latest_chainstate)
    }

    /// Store chainstate to database.
    pub(crate) async fn store_chainstate(
        &mut self,
        new_chainstate: Chainstate,
    ) -> anyhow::Result<Chainstate> {
        let chsman = self.storage.chainstate();
        let block_commitment = new_chainstate.finalized_epoch().to_block_commitment();
        let wb = WriteBatchEntry::new(
            WriteBatch::new(new_chainstate.clone(), Vec::new()),
            block_commitment.blkid().to_owned(),
        );
        chsman
            .put_write_batch_async(new_chainstate.chain_tip_slot(), wb)
            .await?;
        Ok(new_chainstate)
    }

    /// Get latest stored chainstate.
    async fn get_latest_chainstate(&self) -> anyhow::Result<Option<Chainstate>> {
        let chsman = self.storage.chainstate();
        if let Ok(idx) = chsman.get_last_write_idx_async().await {
            let latest_chainstate = chsman
                .get_toplevel_chainstate_async(idx)
                .await?
                .ok_or(DbError::MissingL2State(idx))?
                .to_chainstate();
            return Ok(Some(latest_chainstate));
        }

        Ok(None)
    }
}

pub async fn checkpoint_sync_task(
    storage: Arc<NodeStorage>,
    status_channel: StatusChannel,
) -> anyhow::Result<()> {
    let mut csync_manager = ChainstateStorageInterface::new(storage.clone());

    // initialize target epoch from db
    let mut target_epoch = get_target_epoch(&storage).await?;

    // the only other place where we subscribe to client state is in the FCM
    // which we have disabled during checkpoint sync
    let mut rx = status_channel.subscribe_client_state();

    // wait for CSM to send msg for change in client state
    // TODO: should I use has_changed which will not mark the event as read?
    while rx.changed().await.is_ok() {
        #[cfg(feature = "debug-utils")]
        check_and_pause_debug_async(WorkerType::CheckpointSyncWorker).await;

        let state = rx.borrow().clone(); // will this clone be expensive?
        let Some(l1_checkpoint) = state.get_apparent_finalized_checkpoint() else {
            continue;
        };
        let csm_finalized_epoch = l1_checkpoint.batch_info.epoch;
        let current_epoch = target_epoch.saturating_sub(1);

        if csm_finalized_epoch < current_epoch {
            // is there no viable means to test this yet? since we can not reorg behind finalization
            // depth, this branch is unlikely to be executed
            handle_reorg(
                &mut target_epoch,
                csm_finalized_epoch,
                &mut csync_manager,
                &storage,
                &status_channel,
            )
            .await?;
        } else if csm_finalized_epoch > target_epoch {
            handle_missed_epochs(
                &mut target_epoch,
                csm_finalized_epoch,
                &mut csync_manager,
                &storage,
                &status_channel,
            )
            .await?;
        } else if csm_finalized_epoch == target_epoch {
            handle_target_epoch(
                l1_checkpoint,
                &mut target_epoch,
                csm_finalized_epoch,
                &mut csync_manager,
                &storage,
                &status_channel,
            )
            .await?;
        }
    }

    Ok(())
}

/// Get the epoch one more than that of the latest chainstate available in the database.
/// This is the epoch which will be committed to by the next checkpoint.
async fn get_target_epoch(storage: &Arc<NodeStorage>) -> Result<u64, anyhow::Error> {
    let chsman = storage.chainstate();
    let Some(slot) = chsman.get_last_write_idx_async().await.ok() else {
        return Ok(0);
    };

    match chsman.get_toplevel_chainstate_async(slot).await {
        Ok(Some(entry)) => Ok(entry.state().cur_epoch() + 1),
        Ok(None) | Err(DbError::NotBootstrapped) => Ok(0),
        Err(err) => Err(err.into()),
    }
}

/// Handle chainstate update when the epoch finalized by CSM is greater than the target epoch.
/// This means we must have missed processing previously finalized epochs for chainstate update.
/// ```
/// csm_finalized_epoch > target_epoch
/// ```
async fn handle_missed_epochs(
    target_epoch: &mut u64,
    csm_finalized_epoch: u64,
    csync_manager: &mut ChainstateStorageInterface,
    storage: &NodeStorage,
    status_channel: &StatusChannel,
) -> Result<(), anyhow::Error> {
    warn!(
        %target_epoch, "out-of-order checkpoint finalized: no message received for target epoch",
    );

    // check if checkpoint with target epoch is already present in the database and
    // we simply missed status update from CSM for that finalized epoch
    // need to process a range of epochs here!
    Ok(for epoch in *target_epoch..=csm_finalized_epoch {
        let Some(entry) = storage.checkpoint().get_checkpoint(epoch).await? else {
            error!(
                %epoch, "no checkpoint found for epoch, chainstate not advanced",
            );
            break; // only break the inner for loop
        };

        let tip = entry.checkpoint.batch_info().final_l2_block().clone();
        process_checkpoint_entry(entry, &tip, status_channel, csync_manager).await?;
        *target_epoch += 1; // increment target epoch
    })
}

/// Handle chainstate update when target epoch is the same as the epoch finalized by CSM.
/// ```
/// csm_finalized_epoch == target_epoch
/// ```
async fn handle_target_epoch(
    l1_checkpoint: &L1Checkpoint,
    target_epoch: &mut u64,
    csm_finalized_epoch: u64,
    csync_manager: &mut ChainstateStorageInterface,
    storage: &Arc<NodeStorage>,
    status_channel: &StatusChannel,
) -> Result<(), anyhow::Error> {
    // get checkpoint entry for the latest finalized epoch
    let checkpoint_entry = storage
        .checkpoint()
        .get_checkpoint(csm_finalized_epoch)
        .await?;

    if let Some(entry) = checkpoint_entry {
        process_checkpoint_entry(
            entry,
            l1_checkpoint.batch_info.final_l2_block(),
            status_channel,
            csync_manager,
        )
        .await?;
        *target_epoch += 1; // increment target epoch
    } else {
        // this is a failure branch - if we can't find checkpoint at all in the db there is no way
        // for us to reconstruct it again
        // TODO: add custom error here
        warn!("Didn't find checkpoint entry for CSM finalized epoch");
    }

    Ok(())
}

/// Handle chainstate update when the epoch finalized by CSM is lesser than the current epoch.
/// This means there has been a possible reorg where the latest finalized checkpoint has been rolled
/// back.
///
/// Although this scenario is highly unlikely (since we work within finalization depth limits), and
/// the client also panics if a finalized checkpoint is indeed rolled back (undefined behaviour for
/// a rollup system).
///
/// ```
/// current_epoch = target_epoch.saturating_sub(1)
/// csm_finalized_epoch < current_epoch
/// ```
async fn handle_reorg(
    target_epoch: &mut u64,
    csm_finalized_epoch: u64,
    csync_manager: &mut ChainstateStorageInterface,
    storage: &NodeStorage,
    status_channel: &StatusChannel,
) -> Result<(), anyhow::Error> {
    warn!(
        %csm_finalized_epoch, %target_epoch, "csm finalized epoch is less than target epoch",
    );

    let ckpt_db = storage.checkpoint();
    let Some(latest_epoch_from_db) = ckpt_db.get_last_checkpoint().await? else {
        // CSM hasn't stored any checkpoint to db yet - invalid state!
        return Err(DbError::NotBootstrapped.into());
    };

    Ok(for epoch in csm_finalized_epoch..=latest_epoch_from_db {
        let Some(entry) = ckpt_db.get_checkpoint(epoch).await? else {
            // missing checkpoints in the database for finalized epochs - invalid state!
            error!(
                %epoch, "no checkpoint found for epoch, chainstate not rolled back",
            );
            break; // only break the inner for loop
        };

        // checkpoint confirmation status must be finalized to be processed
        // if entry.confirmation_status ?

        // delete chainstate from previous slot? rollback directly might raise errors!
        // rollback_writes_to function iteratively deletes chainstate from slots -
        // we don't have contiguous range of chainstate!

        let tip = entry.checkpoint.batch_info().final_l2_block();
        process_checkpoint_entry(entry.clone(), tip, status_channel, csync_manager).await?;
        *target_epoch = epoch + 1;
    })
}

/// Process checkpoint entry retrieved from database and send update through status channel.
async fn process_checkpoint_entry(
    entry: CheckpointEntry,
    tip: &L2BlockCommitment,
    status_channel: &StatusChannel,
    csync_manager: &mut ChainstateStorageInterface,
) -> anyhow::Result<()> {
    let latest_chainstate =
        process_and_store_checkpoint(entry.into_batch_checkpoint(), csync_manager).await?;

    send_sync_status_update(latest_chainstate, tip, status_channel);

    Ok(())
}

/// Process a checkpoint entry.
///
/// 1. Extract `ChainstateUpdate` from entry.
/// 2. Apply update to latest chainstate to obtain new chainstate.
/// 3. Store new chainstate to database.
/// 4. Notify status channel receivers through `ChainSyncStatusUpdate`.
async fn process_and_store_checkpoint(
    checkpoint: Checkpoint,
    csync_manager: &mut ChainstateStorageInterface,
) -> anyhow::Result<Chainstate> {
    let chainstate_diff = FullStateUpdate::from_buf(checkpoint.sidecar().bytes())?;

    // apply chainstate update
    let new_chainstate = csync_manager
        .apply_diff_to_latest_chainstate(chainstate_diff)
        .await?;

    let slot = new_chainstate.chain_tip_slot();
    // let epoch = checkpoint.batch_info().epoch;
    // let blkid = new_chainstate.
    // info!(%epoch, %slot, %blkid, "storing updated chainstate");
    info!(%slot, "storing updated chainstate");

    // TODO this might raise an error if we try to overwrite existing chainstate during rollback???
    Ok(csync_manager.store_chainstate(new_chainstate).await?)
}

/// Send `ChainSyncStatusUpdate` message to (potential) listeners through status channel.
fn send_sync_status_update(
    latest_chainstate: Chainstate,
    tip: &L2BlockCommitment,
    status_channel: &StatusChannel,
) {
    let status = ChainSyncStatus::new(
        *tip,
        *latest_chainstate.prev_epoch(),
        *latest_chainstate.finalized_epoch(),
        latest_chainstate.l1_view().get_safe_block(),
    );
    let update = ChainSyncStatusUpdate::new(status, Arc::new(latest_chainstate));
    status_channel.update_chain_sync_status(update);
}

#[cfg(test)]
mod tests {
    use strata_state::{chain_state::FullStateUpdate, traits::ChainstateDiff};
    use strata_test_utils::l2::get_test_signed_checkpoint;

    #[test]
    fn test_extract_chainstate_diff_from_checkpoint() {
        let sc = get_test_signed_checkpoint();
        let ckpt = sc.checkpoint();
        let result = FullStateUpdate::from_buf(ckpt.sidecar().bytes());
        assert!(result.is_ok());
    }
}
