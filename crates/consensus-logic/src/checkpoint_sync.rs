use std::sync::Arc;

#[cfg(feature = "debug-utils")]
use strata_common::{check_and_pause_debug_async, WorkerType};
use strata_db::DbError;
use strata_state::{
    batch::Checkpoint,
    chain_state::{Chainstate, FullStateUpdate},
    client_state::ClientState,
    state_op::{WriteBatch, WriteBatchEntry},
    traits::{ChainstateUpdate, StateUpdateError},
};
use strata_status::{ChainSyncStatus, ChainSyncStatusUpdate, StatusChannel};
use strata_storage::NodeStorage;
use tokio::sync::watch::Receiver;
use tracing::{info, warn};

use crate::errors::{ChainstateValidationError, CheckpointSyncError};

type SyncResult<T> = std::result::Result<T, CheckpointSyncError>;

/// Wrapper around storage operations needed for checkpoint sync.
/// Keeps storage concerns encapsulated.
struct CheckpointSyncStorage {
    storage: Arc<NodeStorage>,
}

impl CheckpointSyncStorage {
    fn new(storage: Arc<NodeStorage>) -> Self {
        Self { storage }
    }

    /// Determine the starting epoch for checkpoint sync by inspecting latest written chainstate.
    async fn init_target_epoch(&self) -> SyncResult<u64> {
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

    /// Fetch the latest stored chainstate.
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

    /// Store chainstate to database.
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

    /// Fetch checkpoint by epoch. Errors if checkpoint isn't found.
    async fn get_checkpoint_by_epoch(&self, epoch: u64) -> SyncResult<Checkpoint> {
        if let Some(entry) = self.storage.checkpoint().get_checkpoint(epoch).await? {
            Ok(entry.into_batch_checkpoint())
        } else {
            Err(CheckpointSyncError::MissingCheckpoint(epoch).into())
        }
    }
}

/// Tracks current state during checkpoint sync: current epoch and working chainstate.
struct CheckpointSyncTracker {
    target_epoch: u64,
    latest_chainstate: Option<Chainstate>,
}

impl CheckpointSyncTracker {
    fn new(target_epoch: u64) -> Self {
        Self {
            target_epoch,
            latest_chainstate: None,
        }
    }

    fn target_epoch(&self) -> u64 {
        self.target_epoch
    }

    fn increment_target_epoch(&mut self) {
        self.target_epoch += 1;
    }

    fn chainstate_mut(&mut self) -> Option<&mut Chainstate> {
        self.latest_chainstate.as_mut()
    }

    fn is_chainstate_initialized(&self) -> bool {
        self.latest_chainstate.is_some()
    }

    fn set_chainstate(&mut self, chainstate: Chainstate) {
        self.latest_chainstate = Some(chainstate);
    }
}

pub async fn checkpoint_sync_task(
    storage: Arc<NodeStorage>,
    status_channel: StatusChannel,
) -> anyhow::Result<()> {
    let mut storage = CheckpointSyncStorage::new(storage);
    let mut sync_tracker = CheckpointSyncTracker::new(storage.init_target_epoch().await?);
    let state_update_extractor = FullStateUpdate::from_buf;
    let mut rx = status_channel.subscribe_client_state();

    loop {
        let finalized_epoch =
            wait_for_epoch_finalization(&mut rx, sync_tracker.target_epoch()).await;

        // initialize chainstate for tracker if needed (only for first loop iteration)
        if !sync_tracker.is_chainstate_initialized() {
            // the CSM will have already initialized genesis in the db so we will at least have the
            // genesis chainstate in db when an epoch is finalized
            let chainstate = storage.get_latest_chainstate().await?;
            sync_tracker.set_chainstate(chainstate);
        }

        for epoch in sync_tracker.target_epoch()..=finalized_epoch {
            if let Some(chainstate) = sync_tracker.chainstate_mut() {
                // process checkpoint corresponding to finalized epoch
                let checkpoint = storage.get_checkpoint_by_epoch(epoch).await?;
                if let Err(e) = process_checkpoint_and_update_chainstate::<FullStateUpdate>(
                    &checkpoint,
                    chainstate,
                    state_update_extractor,
                )
                .await
                {
                    warn!(%epoch, %e, "Failed to process malformed checkpoint; skipping");
                    continue;
                }

                // store updated chainstate
                let slot = chainstate.chain_tip_slot();
                let blkid = checkpoint.batch_info().final_l2_block().blkid();
                let epoch = checkpoint.batch_info().epoch();
                info!(%epoch, %slot, %blkid, "storing updated chainstate");
                storage.store_chainstate(chainstate.clone()).await?;

                // send status update through status channel
                send_status_update(&checkpoint, chainstate, &status_channel);

                // update target epoch
                sync_tracker.increment_target_epoch();
            }
        }
    }
}

/// Wait for target epoch to be finalized.
async fn wait_for_epoch_finalization(rx: &mut Receiver<ClientState>, target_epoch: u64) -> u64 {
    while rx.changed().await.is_ok() {
        #[cfg(feature = "debug-utils")]
        check_and_pause_debug_async(WorkerType::CheckpointSyncWorker).await;

        if let Some(finalized_epoch) = rx.borrow().get_declared_final_epoch().map(|ec| ec.epoch()) {
            if finalized_epoch >= target_epoch {
                return finalized_epoch;
            } else if finalized_epoch < target_epoch.saturating_sub(1) {
                unimplemented!("reorg beyond finalization depth is undefined!");
            }
        }
    }

    // this block executes when receiver drops connection to CSM
    unimplemented!("connection to CSM dropped: error not handled.")
}

/// Publishes the current sync status over the channel.
fn send_status_update(
    checkpoint: &Checkpoint,
    chainstate: &Chainstate,
    status_channel: &StatusChannel,
) {
    let tip = checkpoint.batch_info().final_l2_block();
    let status = ChainSyncStatus::new(
        *tip,
        *chainstate.prev_epoch(),
        *chainstate.finalized_epoch(),
        chainstate.l1_view().get_safe_block(),
    );
    let update = ChainSyncStatusUpdate::new(status, Arc::new(chainstate.clone()));
    status_channel.update_chain_sync_status(update);
}

/// Process a single checkpoint.
/// 1. Extract state update from checkpoint.
/// 2. Apply update to chainstate.
/// 3. Validate the resulting chainstate with checkpoint data.
async fn process_checkpoint_and_update_chainstate<T: ChainstateUpdate>(
    checkpoint: &Checkpoint,
    chainstate: &mut Chainstate,
    extract_state_update: fn(&[u8]) -> Result<T, StateUpdateError>,
) -> SyncResult<()> {
    // can be computationally expensive for now but there is no way of
    // reverting from validation error without keeping a copy
    let pre_update_chainstate = chainstate.clone();

    // extract state update from checkpoint
    let state_update = extract_state_update(checkpoint.aux_data_raw())?;

    // update chainstate
    state_update.apply_to_chainstate(chainstate)?;

    if let Err(e) = validate_chainstate(checkpoint, chainstate) {
        // revert to previous chainstate if error
        *chainstate = pre_update_chainstate;
        return Err(e.into());
    }

    Ok(())
}

/// Validate chainstate according to data committed in checkpoint.
/// Only compares the state roots for now, add futher validation checks as needed.
fn validate_chainstate(
    checkpoint: &Checkpoint,
    chainstate: &Chainstate,
) -> Result<(), ChainstateValidationError> {
    let batch_transition = checkpoint.batch_transition();
    if batch_transition.chainstate_transition.post_state_root != chainstate.compute_state_root() {
        return Err(ChainstateValidationError::StateRootMismatch(
            chainstate.chain_tip_slot(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use strata_test_utils::l2::{gen_params, get_genesis_chainstate, get_test_signed_checkpoint};

    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn test_chainstate_unmodified_for_invalid_checkpoint_data() {
        let params = gen_params();
        let (_, mut chainstate) = get_genesis_chainstate(&params);
        let pre_process_state_root = chainstate.compute_state_root();

        let signed_checkpoint = get_test_signed_checkpoint();
        let checkpoint = signed_checkpoint.checkpoint();
        let state_update_extractor = FullStateUpdate::from_buf;
        let verification_result = process_checkpoint_and_update_chainstate::<FullStateUpdate>(
            checkpoint,
            &mut chainstate,
            state_update_extractor,
        )
        .await;

        let post_process_state_root = chainstate.compute_state_root();

        if verification_result.is_err() {
            assert_eq!(pre_process_state_root, post_process_state_root);
        } else {
            assert_ne!(pre_process_state_root, post_process_state_root);
        }
    }
}
