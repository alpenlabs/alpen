use std::sync::Arc;

use strata_consensus_logic::checkpoint_sync::CheckpointSyncManager;
use strata_db::types::CheckpointEntry;
use strata_state::{da::ChainstateDAScheme, traits::ChainstateDA};
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
        if csm_finalized_epoch < target_epoch.saturating_sub(1) {
            // this means there was a reorg where the current epoch was rolled back
            let Some(entry) = ckpt_db.get_checkpoint(csm_finalized_epoch).await? else {
                error!("failed to rollback");
                continue;
            };
            process_checkpoint_entry(entry, l1_checkpoint, &status_channel, &mut csync_manager)
                .await;
            target_epoch = csm_finalized_epoch + 1; // reset state to as defined by latest finalized
                                                    // checkpoint
        } else if csm_finalized_epoch > target_epoch {
            // something went wrong and we missed processing checkpoint transaction for target epoch
            warn!(
                "out of order checkpoint finalized: no message received for target epoch: {}",
                target_epoch,
            );

            // check if checkpoint with target epoch is already present in the database and
            // we simply missed status update from CSM for that finalized epoch
            // need to process a range of epochs here!
            for epoch in target_epoch..=csm_finalized_epoch {
                let Some(entry) = ckpt_db.get_checkpoint(epoch).await? else {
                    error!(
                        "no checkpoint found for epoch : {}, chainstate not advanced",
                        epoch
                    );
                    break; // only break the inner for loop
                };
                process_checkpoint_entry(entry, l1_checkpoint, &status_channel, &mut csync_manager)
                    .await;
                target_epoch += 1; // increment target epoch for each checkpoint processed
            }
        } else if csm_finalized_epoch == target_epoch {
            // get checkpoint entry for the latest finalized epoch
            let checkpoint_entry = storage
                .checkpoint()
                .get_checkpoint(csm_finalized_epoch)
                .await;

            if let Ok(Some(entry)) = checkpoint_entry {
                process_checkpoint_entry(
                    entry,
                    l1_checkpoint, // we can pass l2 block commitment here
                    &status_channel,
                    &mut csync_manager,
                )
                .await;
                target_epoch += 1; // increment target epoch
            }
        }
    }

    Ok(())
}

/// Process a checkpoint entry.
///
/// 1. Extract `ChainstateUpdate` from entry.
/// 2. Apply update to latest chainstate to obtain new chainstate.
/// 3. Store new chainstate to database.
/// 4. Notify status channel receivers through `ChainSyncStatusUpdate`.
async fn process_checkpoint_entry(
    entry: CheckpointEntry,
    l1_checkpoint: &strata_state::client_state::L1Checkpoint,
    status_channel: &StatusChannel,
    csync_manager: &mut CheckpointSyncManager,
) {
    if let Ok(chainstate_update) =
        ChainstateDAScheme::chainstate_update_from_bytes(entry.checkpoint.sidecar().bytes())
    {
        // apply chainstate update
        if let Ok(new_chainstate) = csync_manager
            .apply_chainstate_update(chainstate_update)
            .await
        {
            info!(
                "storing updated chainstate, slot: {}",
                new_chainstate.chain_tip_slot()
            );

            // store chainstate to database
            let latest_chainstate = match csync_manager.store_chainstate(new_chainstate).await {
                Ok(chainstate) => chainstate,
                Err(err) => {
                    warn!("failed to store chainstate: {}", err);
                    return;
                }
            };

            // submit event of chainstate update
            let status = ChainSyncStatus::new(
                *l1_checkpoint.batch_info.final_l2_block(),
                *latest_chainstate.prev_epoch(),
                *latest_chainstate.finalized_epoch(),
                latest_chainstate.l1_view().get_safe_block(),
            );
            let update = ChainSyncStatusUpdate::new(status, Arc::new(latest_chainstate));
            status_channel.update_chain_sync_status(update);
        }
    }
}
