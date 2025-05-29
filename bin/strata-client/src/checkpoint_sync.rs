use std::sync::Arc;

use strata_consensus_logic::checkpoint_sync::CheckpointSyncManager;
use strata_state::{da::ChainstateDAScheme, traits::ChainstateDA};
use strata_status::{ChainSyncStatus, ChainSyncStatusUpdate, StatusChannel};
use strata_storage::NodeStorage;
use tracing::{info, warn};

pub async fn checkpoint_sync_task(
    storage: Arc<NodeStorage>,
    status_channel: StatusChannel,
) -> anyhow::Result<()> {
    let mut csync_manager = CheckpointSyncManager::new(storage.clone());
    let mut target_epoch = 0;

    // the only other place where we subscribe to client state is in the FCM
    // which we have disabled during checkpoint sync
    let mut rx = status_channel.subscribe_client_state();

    // wait for CSM to send msg for change in client state
    // TODO: should I use has_changed which will not mark the event as read?
    while rx.changed().await.is_ok() {
        let state = rx.borrow().clone(); // will this clone be expensive?
        let Some(ckpt) = state.get_apparent_finalized_checkpoint() else {
            continue;
        };

        let latest_finalized_epoch = ckpt.batch_info.epoch;

        // TODO: also check if latest_finalized_epoch is lesser than target epoch
        // in which case it is a reorg
        if latest_finalized_epoch != target_epoch {
            continue;
        }

        // get checkpoint entry for the latest finalized epoch
        let checkpoint_entry = storage
            .checkpoint()
            .get_checkpoint(latest_finalized_epoch)
            .await;

        if let Ok(Some(entry)) = checkpoint_entry {
            if let Ok(chainstate_update) =
                ChainstateDAScheme::chainstate_update_from_bytes(entry.checkpoint.sidecar().bytes())
            {
                // apply chainstate update
                let chainstate_result = csync_manager
                    .apply_chainstate_update(chainstate_update)
                    .await;

                if let Ok(cs) = chainstate_result {
                    info!("storing updated chainstate, slot: {}", cs.chain_tip_slot());

                    // store chainstate to database
                    let cs = match csync_manager.store_chainstate(cs).await {
                        Ok(chainstate) => chainstate,
                        Err(err) => {
                            warn!("failed to store chainstate, aborting sync: {}", err);
                            break;
                        }
                    };

                    // submit event of chainstate update
                    let status = ChainSyncStatus::new(
                        *ckpt.batch_info.final_l2_block(),
                        *cs.prev_epoch(),
                        *cs.finalized_epoch(),
                        cs.l1_view().get_safe_block(),
                    );
                    let update = ChainSyncStatusUpdate::new(status, Arc::new(cs));
                    status_channel.update_chain_sync_status(update);

                    // increment target epoch
                    target_epoch += 1;
                }
            }
        }
    }

    Ok(())
}
