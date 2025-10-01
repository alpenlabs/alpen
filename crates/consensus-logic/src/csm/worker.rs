//! CSM (Client State Machine) listener service.
//!
//! This service listens to ASM worker status updates and processes checkpoint logs
//! emitted by the checkpoint-v0 subprotocol. It maintains the client state by
//! reacting to checkpoint updates rather than scanning L1 blocks directly.

use std::sync::Arc;

use serde::Serialize;
use strata_asm_common::AsmLogEntry;
use strata_asm_logs::{constants::CHECKPOINT_UPDATE_LOG_TYPE, CheckpointUpdate};
use strata_asm_worker::AsmWorkerStatus;
use strata_checkpoint_types::{BatchTransition, Checkpoint, CheckpointSidecar, SignedCheckpoint};
use strata_csm_types::{
    CheckpointL1Ref, ClientState, ClientUpdateOutput, L1Checkpoint, SyncAction,
};
use strata_db::types::{CheckpointConfStatus, CheckpointEntry, CheckpointProvingStatus};
use strata_primitives::{buf::Buf64, prelude::*};
use strata_service::{Response, Service, ServiceState, SyncService};
use strata_status::StatusChannel;
use strata_storage::NodeStorage;
use tracing::*;

/// CSM listener service that listens to ASM worker status updates.
///
/// This service reacts to checkpoint logs emitted by the checkpoint-v0 subprotocol
/// in ASM. When ASM processes a checkpoint transaction, it emits a CheckpointUpdate
/// log which this service processes to update the client state.
#[derive(Debug)]
pub struct CsmListenerService;

impl Service for CsmListenerService {
    type State = CsmListenerState;
    type Msg = AsmWorkerStatus;
    type Status = CsmListenerStatus;

    fn get_status(state: &Self::State) -> Self::Status {
        CsmListenerStatus {
            cur_block: state.last_asm_block,
            last_processed_epoch: state.last_processed_epoch,
        }
    }
}

impl SyncService for CsmListenerService {
    fn process_input(state: &mut Self::State, asm_status: &Self::Msg) -> anyhow::Result<Response> {
        // Extract the current block from ASM status
        let Some(asm_block) = asm_status.cur_block else {
            // ASM hasn't processed any blocks yet
            trace!("ASM status has no current block, skipping");
            return Ok(Response::Continue);
        };

        // Track which block we're processing
        state.last_asm_block = Some(asm_block);

        // Extract checkpoint logs from ASM status
        let logs = asm_status.logs();

        if logs.is_empty() {
            trace!(%asm_block, "No logs in ASM status update");
            return Ok(Response::Continue);
        }

        // Process each checkpoint update log
        for log in logs {
            if let Err(e) = process_checkpoint_log(state, log, &asm_block) {
                error!(%asm_block, err = %e, "Failed to process checkpoint log");
                // Continue processing other logs instead of failing completely
                // Or should we?
            }
        }

        Ok(Response::Continue)
    }
}

/// Process a single ASM log entry, extracting and handling checkpoint updates.
fn process_checkpoint_log(
    state: &mut CsmListenerState,
    log: &AsmLogEntry,
    asm_block: &L1BlockCommitment,
) -> anyhow::Result<()> {
    // Check if this is a checkpoint update log
    if log.ty() != Some(CHECKPOINT_UPDATE_LOG_TYPE) {
        trace!("Skipping non-checkpoint log");
        return Ok(());
    }

    // Deserialize the checkpoint update using the AsmLog trait
    let checkpoint_update: CheckpointUpdate = log
        .try_into_log()
        .map_err(|e| anyhow::anyhow!("Failed to deserialize CheckpointUpdate: {}", e))?;

    let epoch = checkpoint_update.batch_info.epoch();

    info!(
        %epoch,
        %asm_block,
        checkpoint_txid = ?checkpoint_update.checkpoint_txid,
        "Processing checkpoint update from ASM log"
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
        checkpoint: create_signed_checkpoint_from_update(&checkpoint_update).into(),
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
    state: &mut CsmListenerState,
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
            // First checkpoint becomes recent
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

/// Create a SignedCheckpoint from a CheckpointUpdate log.
///
/// Note: The log doesn't contain the full signed checkpoint, so we reconstruct
/// what we can. The signature verification was already done by ASM.
fn create_signed_checkpoint_from_update(update: &CheckpointUpdate) -> SignedCheckpoint {
    let epoch = update.batch_info.epoch();

    // Create empty sidecar - checkpoint was already verified by ASM
    let sidecar = CheckpointSidecar::new(vec![]);

    let checkpoint = Checkpoint::new(
        update.batch_info.clone(),
        BatchTransition {
            epoch,
            chainstate_transition: update.chainstate_transition,
        },
        Default::default(), // Empty proof - actual proof was already verified by ASM
        sidecar,
    );

    // Create a signed checkpoint with empty signature since ASM already verified it
    SignedCheckpoint::new(checkpoint, Buf64::zero())
}

/// State for the CSM listener service.
#[expect(
    missing_debug_implementations,
    reason = "NodeStorage doesn't implement Debug"
)]
pub struct CsmListenerState {
    /// Consensus parameters.
    _params: Arc<Params>,

    /// Node storage handle.
    storage: Arc<NodeStorage>,

    /// Current client state.
    cur_state: Arc<ClientState>,

    /// Last ASM block we processed.
    last_asm_block: Option<L1BlockCommitment>,

    /// Last epoch we processed a checkpoint for.
    last_processed_epoch: Option<u64>,

    /// Status channel for publishing state updates.
    status_channel: StatusChannel,
}

impl CsmListenerState {
    /// Create a new CSM listener state.
    pub fn new(
        params: Arc<Params>,
        storage: Arc<NodeStorage>,
        status_channel: StatusChannel,
    ) -> anyhow::Result<Self> {
        // Load the most recent client state from storage
        let (cur_block, cur_state) = storage
            .client_state()
            .fetch_most_recent_state()?
            .expect("missing initial client state?");

        Ok(Self {
            _params: params,
            storage,
            cur_state: Arc::new(cur_state),
            last_asm_block: Some(cur_block),
            last_processed_epoch: None,
            status_channel,
        })
    }
}

impl ServiceState for CsmListenerState {
    fn name(&self) -> &str {
        "csm_listener"
    }
}

/// Status information for the CSM listener service.
#[derive(Clone, Debug, Serialize)]
pub struct CsmListenerStatus {
    pub cur_block: Option<L1BlockCommitment>,
    pub last_processed_epoch: Option<u64>,
}

/// Apply a sync action to storage.
fn apply_action(action: SyncAction, storage: &Arc<NodeStorage>) -> anyhow::Result<()> {
    let ckpt_db = storage.checkpoint();
    match action {
        SyncAction::FinalizeEpoch(epoch_comm) => {
            // For the fork choice manager this gets picked up later.  We don't have
            // to do anything here *necessarily*.
            info!(?epoch_comm, "finalizing epoch");

            strata_common::check_bail_trigger("sync_event_finalize_epoch");

            // Write that the checkpoint is finalized.
            //
            // TODO In the future we should just be able to determine this on the fly.
            let epoch = epoch_comm.epoch();
            let Some(mut ckpt_entry) = ckpt_db.get_checkpoint_blocking(epoch)? else {
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

            ckpt_db.put_checkpoint_blocking(epoch, ckpt_entry)?;
        }

        // Update checkpoint entry in database to mark it as included in L1.
        SyncAction::UpdateCheckpointInclusion {
            checkpoint,
            l1_reference,
        } => {
            let epoch = checkpoint.batch_info().epoch();

            let mut ckpt_entry = match ckpt_db.get_checkpoint_blocking(epoch)? {
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

            ckpt_db.put_checkpoint_blocking(epoch, ckpt_entry)?;
        }
    }

    Ok(())
}
