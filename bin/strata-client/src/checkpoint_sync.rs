use std::sync::Arc;

use strata_consensus_logic::checkpoint_sync::CheckpointSyncManager;
use strata_db::types::CheckpointEntry;
use strata_primitives::l2::L2BlockCommitment;
use strata_state::{
    batch::Checkpoint, chain_state::Chainstate, client_state::L1Checkpoint, da::ChainstateDAScheme,
    traits::ChainstateDA,
};
use strata_status::{ChainSyncStatus, ChainSyncStatusUpdate, StatusChannel};
use strata_storage::NodeStorage;
use tracing::{error, info, warn};

// TODO: need a retry wrapper so that we don't run an indefinite loop
// or at least exponential backoff for each failed attempt to sync

pub async fn checkpoint_sync_task(
    storage: Arc<NodeStorage>,
    status_channel: StatusChannel,
) -> anyhow::Result<()> {
    let mut csync_manager = CheckpointSyncManager::new(storage.clone());

    // initialize target epoch from db
    // some checkpoints might have remained unprocessed - do I need to compare latest chainstate
    // with checkpoint stored in db first?
    let ckpt_db = storage.checkpoint();
    let mut target_epoch = ckpt_db
        .get_last_checkpoint()
        .await?
        .map_or(0, |epoch| epoch + 1);

    // the only other place where we subscribe to client state is in the FCM
    // which we have disabled during checkpoint sync
    let mut rx = status_channel.subscribe_client_state();

    // wait for CSM to send msg for change in client state
    // TODO: should I use has_changed which will not mark the event as read?
    while rx.changed().await.is_ok() {
        let state = rx.borrow().clone(); // will this clone be expensive?
        let Some(l1_checkpoint) = state.get_apparent_finalized_checkpoint() else {
            continue;
        };
        let csm_finalized_epoch = l1_checkpoint.batch_info.epoch;
        let current_epoch = target_epoch.saturating_sub(1);

        if csm_finalized_epoch < current_epoch {
            handle_reorg(
                l1_checkpoint,
                &mut target_epoch,
                csm_finalized_epoch,
                &mut csync_manager,
                &storage,
                &status_channel,
            )
            .await?;
        } else if csm_finalized_epoch > target_epoch {
            handle_missed_checkpoints(
                l1_checkpoint,
                &mut target_epoch,
                csm_finalized_epoch,
                &mut csync_manager,
                &storage,
                &status_channel,
            )
            .await?;
        } else {
            handle_l1_checkpoint(
                l1_checkpoint,
                &mut target_epoch,
                csm_finalized_epoch,
                &mut csync_manager,
                &storage,
                &status_channel,
            )
            .await;
        }
    }

    Ok(())
}

async fn handle_missed_checkpoints(
    l1_checkpoint: &L1Checkpoint,
    target_epoch: &mut u64,
    csm_finalized_epoch: u64,
    csync_manager: &mut CheckpointSyncManager,
    storage: &NodeStorage,
    status_channel: &StatusChannel,
) -> Result<(), anyhow::Error> {
    // TODO how do I test this branch?
    // something went wrong and we missed processing checkpoint transaction for target epoch
    warn!(
        "out of order checkpoint finalized: no message received for target epoch: {}",
        target_epoch,
    );

    // check if checkpoint with target epoch is already present in the database and
    // we simply missed status update from CSM for that finalized epoch
    // need to process a range of epochs here!
    Ok(for epoch in *target_epoch..=csm_finalized_epoch {
        let Some(entry) = storage.checkpoint().get_checkpoint(epoch).await? else {
            error!(
                "no checkpoint found for epoch : {}, chainstate not advanced",
                epoch
            );
            break; // only break the inner for loop
        };

        if process_checkpoint_entry(
            entry,
            l1_checkpoint.batch_info.final_l2_block(),
            status_channel,
            csync_manager,
        )
        .await
        .is_ok()
        {
            *target_epoch += 1; // increment target epoch
        }
    })
}

async fn handle_l1_checkpoint(
    l1_checkpoint: &L1Checkpoint,
    target_epoch: &mut u64,
    csm_finalized_epoch: u64,
    csync_manager: &mut CheckpointSyncManager,
    storage: &Arc<NodeStorage>,
    status_channel: &StatusChannel,
) {
    // get checkpoint entry for the latest finalized epoch
    let checkpoint_entry = storage
        .checkpoint()
        .get_checkpoint(csm_finalized_epoch)
        .await;

    if let Ok(Some(entry)) = checkpoint_entry {
        if process_checkpoint_entry(
            entry,
            l1_checkpoint.batch_info.final_l2_block(),
            status_channel,
            csync_manager,
        )
        .await
        .is_ok()
        {
            *target_epoch += 1; // increment target epoch
        }
    }
}

async fn handle_reorg(
    l1_checkpoint: &L1Checkpoint,
    target_epoch: &mut u64,
    csm_finalized_epoch: u64,
    csync_manager: &mut CheckpointSyncManager,
    storage: &NodeStorage,
    status_channel: &StatusChannel,
) -> Result<(), anyhow::Error> {
    let ckpt_db = storage.checkpoint();

    // will this be the last entry stored into the database or simply the checkpoint with the
    // greatest epoch? DO NOT unwrap here!!!
    let latest_epoch_from_db = ckpt_db.get_last_checkpoint().await?.unwrap();

    Ok(for epoch in csm_finalized_epoch..=latest_epoch_from_db {
        let Some(entry) = ckpt_db.get_checkpoint(epoch).await? else {
            error!(
                "no checkpoint found for epoch : {}, chainstate not rolled back",
                epoch
            );
            break; // only break the inner for loop
        };

        // checkpoint confirmation status must be finalized to be processed
        // if entry.confirmation_status

        // delete chainstate from previous slot? rollback directly might raise errors!
        // rollback_writes_to function iteratively deletes chainstate from slots -
        // we don't have contiguous range of chainstate!

        if process_checkpoint_entry(
            entry,
            l1_checkpoint.batch_info.final_l2_block(),
            status_channel,
            csync_manager,
        )
        .await
        .is_ok()
        {
            *target_epoch = epoch + 1;
        }
    })
}

/// Process checkpoint entry retrieved from database and send update through status channel.
async fn process_checkpoint_entry(
    entry: CheckpointEntry,
    tip: &L2BlockCommitment,
    status_channel: &StatusChannel,
    csync_manager: &mut CheckpointSyncManager,
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
    csync_manager: &mut CheckpointSyncManager,
) -> anyhow::Result<Chainstate> {
    let chainstate_update =
        ChainstateDAScheme::chainstate_update_from_bytes(checkpoint.sidecar().bytes())?;

    // apply chainstate update
    let new_chainstate = csync_manager
        .apply_chainstate_update(chainstate_update)
        .await?;

    info!(
        "storing updated chainstate, slot: {}",
        new_chainstate.chain_tip_slot()
    );

    // TODO this might raise an error if we try to overwrite existing chainstate during rollback???
    Ok(csync_manager.store_chainstate(new_chainstate).await?)
}

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
