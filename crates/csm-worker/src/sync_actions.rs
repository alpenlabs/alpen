//! Sync action application logic.

use std::sync::Arc;

use strata_csm_types::SyncAction;
use strata_db_types::types::{CheckpointConfStatus, CheckpointEntry, CheckpointProvingStatus};
use strata_storage::NodeStorage;
use tracing::*;

/// Apply a sync action to storage.
pub(crate) fn apply_action(action: SyncAction, storage: &Arc<NodeStorage>) -> anyhow::Result<()> {
    let ckpt_db = storage.checkpoint();
    match action {
        SyncAction::FinalizeEpoch(epoch_comm) => {
            info!(?epoch_comm, "finalizing epoch");

            strata_common::check_bail_trigger("csm_event_finalize_epoch");

            // Write that the checkpoint is finalized.
            //
            // TODO In the future we should just be able to determine this on the fly.
            let epoch = epoch_comm.epoch();
            let Some(mut ckpt_entry) = ckpt_db.get_checkpoint_blocking(epoch as u64)? else {
                warn!(%epoch, "missing checkpoint we wanted to mark confirmed, ignoring");
                return Ok(());
            };

            let CheckpointConfStatus::Confirmed(l1ref) = ckpt_entry.confirmation_status else {
                warn!(
                    ?epoch_comm,
                    ?ckpt_entry.confirmation_status,
                    "Expected epoch checkpoint to be confirmed in db, but has different status"
                );
                return Ok(());
            };

            debug!(%epoch, "Marking checkpoint as finalized");
            // Mark it as finalized.
            ckpt_entry.confirmation_status = CheckpointConfStatus::Finalized(l1ref);

            ckpt_db.put_checkpoint_blocking(epoch as u64, ckpt_entry)?;
        }

        // Update checkpoint entry in database to mark it as included in L1.
        SyncAction::UpdateCheckpointInclusion {
            checkpoint,
            l1_reference,
        } => {
            let epoch = checkpoint.batch_info().epoch();

            let mut ckpt_entry = match ckpt_db.get_checkpoint_blocking(epoch as u64)? {
                Some(c) => c,
                None => {
                    info!(%epoch, "creating new checkpoint entry since the database does not have one");

                    CheckpointEntry::new(
                        checkpoint,
                        CheckpointProvingStatus::ProofReady,
                        CheckpointConfStatus::Pending,
                    )
                }
            };

            ckpt_entry.confirmation_status = CheckpointConfStatus::Confirmed(l1_reference);

            ckpt_db.put_checkpoint_blocking(epoch as u64, ckpt_entry)?;
        }
    }

    Ok(())
}
