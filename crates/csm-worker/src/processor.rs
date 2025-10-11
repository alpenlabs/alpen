//! Checkpoint log processing logic.

use std::sync::Arc;

use strata_asm_common::AsmLogEntry;
use strata_asm_logs::{CheckpointUpdate, constants::CHECKPOINT_UPDATE_LOG_TYPE};
use strata_checkpoint_types::{BatchTransition, Checkpoint, CheckpointSidecar};
use strata_csm_types::{
    CheckpointL1Ref, ClientState, ClientUpdateOutput, L1Checkpoint, SyncAction,
};
use strata_primitives::prelude::*;
use tracing::*;

use crate::{state::CsmWorkerState, sync_actions::apply_action};

pub(crate) fn process_logs(
    state: &mut CsmWorkerState,
    log: &AsmLogEntry,
    asm_block: &L1BlockCommitment,
) -> anyhow::Result<()> {
    match log.ty() {
        Some(CHECKPOINT_UPDATE_LOG_TYPE) => return process_checkpoint_log(state, log, asm_block),
        Some(log_type) => {
            warn!(log_type, "not yet supported");
        }
        None => {
            warn!("logs without a type ID?");
        }
    }
    Ok(())
}

/// Process a single ASM log entry, extracting and handling checkpoint updates.
fn process_checkpoint_log(
    state: &mut CsmWorkerState,
    log: &AsmLogEntry,
    asm_block: &L1BlockCommitment,
) -> anyhow::Result<()> {
    // Assert that the dispatch is fine.
    assert_eq!(log.ty(), Some(CHECKPOINT_UPDATE_LOG_TYPE));

    // Deserialize the checkpoint update using the AsmLog trait
    let checkpoint_update: CheckpointUpdate = log
        .try_into_log()
        .map_err(|e| anyhow::anyhow!("Failed to deserialize CheckpointUpdate: {}", e))?;

    let epoch = checkpoint_update.batch_info.epoch();

    info!(
        %epoch,
        %asm_block,
        checkpoint_txid = ?checkpoint_update.checkpoint_txid,
        "CSM is processing checkpoint update from ASM log"
    );

    // Create L1 checkpoint reference from the log data
    let l1_reference = CheckpointL1Ref::new(
        *asm_block,
        checkpoint_update.checkpoint_txid.inner_raw(),
        checkpoint_update.checkpoint_txid.inner_raw(), // TODO: get wtxid if available
    );

    // Create L1Checkpoint for client state
    let l1_checkpoint = L1Checkpoint::new(
        checkpoint_update.batch_info.clone(),
        BatchTransition {
            epoch,
            chainstate_transition: checkpoint_update.chainstate_transition,
        },
        l1_reference.clone(),
    );

    // Update the client state with this checkpoint
    update_client_state_with_checkpoint(state, l1_checkpoint, epoch)?;

    // Create sync action to update checkpoint entry in database
    let sync_action = SyncAction::UpdateCheckpointInclusion {
        checkpoint: create_checkpoint_from_update(&checkpoint_update),
        l1_reference,
    };

    // Apply the sync action
    apply_action(sync_action, &state.storage)?;

    // Track the last processed epoch
    state.last_processed_epoch = Some(epoch);

    Ok(())
}

/// Update client state with a new checkpoint.
fn update_client_state_with_checkpoint(
    state: &mut CsmWorkerState,
    new_checkpoint: L1Checkpoint,
    epoch: u64,
) -> anyhow::Result<()> {
    // Get the current client state
    let cur_state = state.cur_state.as_ref();

    // Determine if this checkpoint should be the last finalized or just recent
    let (last_finalized, recent) = match cur_state.get_last_checkpoint() {
        Some(existing) => {
            // If the new checkpoint is for a later epoch, it becomes recent
            if epoch > existing.batch_info.epoch() {
                (Some(existing.clone()), Some(new_checkpoint))
            } else {
                // Otherwise keep existing
                (Some(existing.clone()), None)
            }
        }
        None => {
            // New checkpoint is the first checkpoint, and it is marked recent
            (None, Some(new_checkpoint))
        }
    };

    // Create new client state
    let next_state = ClientState::new(last_finalized, recent.clone());

    // Check if we need to finalize an epoch
    let old_final_epoch = cur_state.get_declared_final_epoch();
    let new_final_epoch = next_state.get_declared_final_epoch();

    let should_finalize = match (old_final_epoch, new_final_epoch) {
        (None, Some(new)) => Some(new),
        (Some(old), Some(new)) if new.epoch() > old.epoch() => Some(new),
        _ => None,
    };

    // Store the new client state
    let l1_block = state.last_asm_block.expect("should have ASM block");
    state.storage.client_state().put_update_blocking(
        &l1_block,
        ClientUpdateOutput::new(next_state.clone(), vec![]),
    )?;

    // Update our tracked state
    state.cur_state = Arc::new(next_state);

    // Update status channel
    state
        .status_channel
        .update_client_state(state.cur_state.as_ref().clone(), l1_block);

    // Handle epoch finalization if needed
    if let Some(epoch_comm) = should_finalize {
        info!(?epoch_comm, "Finalizing epoch from checkpoint");
        apply_action(SyncAction::FinalizeEpoch(epoch_comm), &state.storage)?;
    }

    Ok(())
}

/// Create a [`Checkpoint`] from a [`CheckpointUpdate`] log.
///
/// Note: The log doesn't contain the full signed checkpoint, so we reconstruct
/// what we can. The signature verification was already done by ASM.
///
/// TODO: This function is created for compatibility reason to avoid making larger changes.
/// This will be largely changed as we move to the new OL STF as the checkpoint structure
/// will be different than the existing ones.
fn create_checkpoint_from_update(update: &CheckpointUpdate) -> Checkpoint {
    let epoch = update.batch_info.epoch();

    // Create empty sidecar - checkpoint was already verified by ASM
    let sidecar = CheckpointSidecar::new(vec![]);

    Checkpoint::new(
        update.batch_info.clone(),
        BatchTransition {
            epoch,
            chainstate_transition: update.chainstate_transition,
        },
        Default::default(), // Empty proof - actual proof was already verified by ASM
        sidecar,
    )
}
