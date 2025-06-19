use std::sync::Arc;

use strata_db::types::CheckpointConfStatus;
use strata_state::{chain_state::FullStateUpdate, traits::ChainstateDiff};
use strata_storage::NodeStorage;
use tracing::{info, warn};

use crate::errors::{CheckpointError, SyncError};

pub async fn snap_sync_task(storage: Arc<NodeStorage>) -> Result<(), SyncError> {
    let ckpt_db = storage.checkpoint();
    let chs_db = storage.chainstate();
    let extract_diff = FullStateUpdate::from_buf;

    let latest_epoch = ckpt_db.get_last_checkpoint().await?.unwrap();
    for epoch in (0..=latest_epoch).rev() {
        let checkpoint_entry = ckpt_db
            .get_checkpoint(epoch)
            .await?
            .ok_or(CheckpointError::MissingCheckpoint(epoch))?;

        if !matches!(
            checkpoint_entry.confirmation_status,
            CheckpointConfStatus::Finalized(_)
        ) {
            continue; // only handle snap sync for checkpoints that are confirmed
        }

        let checkpoint = checkpoint_entry.into_batch_checkpoint();
        let slot = checkpoint.batch_info().final_l2_slot();
        let chainstate = match chs_db.get_toplevel_chainstate_async(slot).await? {
            Some(entry) => entry.to_chainstate(),
            None => continue, // chainstate not found so continue range
        };

        let l1_state_root = extract_diff(checkpoint.sidecar().bytes())?.state_root();
        let local_state_root = chainstate.compute_state_root();
        if l1_state_root == local_state_root {
            info!(%slot, "chainstate snap synced");

            // purge chainstate entries after the confirmed slot since those are not validated yet
            // is this even necessary? what do we do after validating local chainstate?
            purge_chainstate_entires_after_slot(slot).await?;

            return Ok(());
        }
    }

    warn!("couldn't snap sync to L1: no state root in local matches that posted on L1");

    Ok(())
}

// not implemented yet
async fn purge_chainstate_entires_after_slot(_slot: u64) -> anyhow::Result<()> {
    Ok(())
}
