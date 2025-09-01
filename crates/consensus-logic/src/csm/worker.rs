//! Consensus logic worker task.

use std::{sync::Arc, thread};

use strata_db::types::{CheckpointConfStatus, CheckpointEntry, CheckpointProvingStatus};
use strata_primitives::prelude::*;
use strata_state::{
    client_state::{ClientState, ClientStateMut},
    operation::{ClientUpdateOutput, SyncAction},
};
use strata_status::StatusChannel;
use strata_storage::NodeStorage;
use strata_tasks::ShutdownGuard;
use tokio::{
    sync::{broadcast, mpsc},
    time,
};
use tracing::*;

use super::{client_transition, config::CsmExecConfig};
use crate::{errors::Error, genesis};

/// Mutable worker state that we modify in the consensus worker task.
///
/// Unable to be shared across threads.  Any data we want to export we'll do
/// through another handle.
#[expect(missing_debug_implementations)]
pub struct WorkerState {
    /// Consensus parameters.
    params: Arc<Params>,

    /// CSM worker config, *not* params.
    config: CsmExecConfig,

    /// Node storage handle.
    storage: Arc<NodeStorage>,

    /// Current state ref.
    cur_state: Arc<ClientState>,

    // TODO(QQ): Ideally height for cur_state should be here.
    last_height: u64,
}

impl WorkerState {
    /// Constructs a new instance by reconstructing the current consensus state
    /// from the provided database layer.
    pub fn open(params: Arc<Params>, storage: Arc<NodeStorage>) -> anyhow::Result<Self> {
        let cur_state = storage.client_state()._get_most_recent_state_blocking();

        // TODO make configurable
        let config = CsmExecConfig {
            retry_base_dur: time::Duration::from_millis(1000),
            // These settings makes the last retry delay be 6 seconds.
            retry_cnt_max: 20,
            retry_backoff_mult: 1120,
        };
        let last_height = cur_state.height();

        Ok(Self {
            params,
            config,
            storage,
            cur_state,
            last_height,
        })
    }

    /// Gets a ref to the consensus state from the inner state tracker.
    pub fn cur_state(&self) -> &Arc<ClientState> {
        &self.cur_state
    }

    /// Given the next event index, computes the state application if the
    /// requisite data is available.  Returns the output and the new state.
    ///
    /// This is copied from the old `StateTracker` type which we removed to
    /// simplify things.
    // TODO maybe remove output return value
    pub fn advance_consensus_state(
        &mut self,
        block: &L1BlockCommitment,
    ) -> anyhow::Result<ClientUpdateOutput> {
        debug!(%block, "processing sync event");

        // Compute the state transition.
        let context = client_transition::StorageEventContext::new(&self.storage);
        let mut state_mut = ClientStateMut::new(self.cur_state.as_ref().clone());
        client_transition::process_event(&mut state_mut, block, &context, &self.params)?;

        // Clone the state and apply the operations to it.
        let outp = state_mut.into_update();

        Ok(outp)
    }

    fn update_bookeeping(&mut self, last_height: u64, state: Arc<ClientState>) {
        debug!(%last_height, ?state, "computed new consensus state");
        self.cur_state = state;
        self.last_height = last_height;
    }
}

/// Receives messages from channel to update consensus state with.
// TODO consolidate all these channels into container/"io" types
pub fn client_worker_task(
    shutdown: ShutdownGuard,
    mut state: WorkerState,
    mut block_rx: mpsc::Receiver<L1BlockCommitment>,
    status_channel: StatusChannel,
) -> anyhow::Result<()> {
    info!("started CSM worker");

    while let Some(msg) = block_rx.blocking_recv() {
        if let Err(e) = process_msg(&mut state, &msg, &status_channel, &shutdown) {
            error!(err = %e, ?msg, "failed to process sync message, aborting!");
            return Err(e);
        }

        if shutdown.should_shutdown() {
            warn!("received shutdown signal");
            break;
        }
    }

    info!("consensus task exiting");

    Ok(())
}

fn process_msg(
    state: &mut WorkerState,
    block: &L1BlockCommitment,
    status_channel: &StatusChannel,
    shutdown: &ShutdownGuard,
) -> anyhow::Result<()> {
    strata_common::check_bail_trigger("sync_event");

    // FIXME: We should be explicit about what errors to retry instead of just
    // retrying whenever this fails.
    handle_new_block_with_retry(state, block, shutdown, status_channel)?;

    Ok(())
}

/// Repeatedly calls `handle_sync_event`, retrying on failure, up to a limit
/// after which we return with the most recent error.
fn handle_new_block_with_retry(
    state: &mut WorkerState,
    block: &L1BlockCommitment,
    shutdown: &ShutdownGuard,
    status_channel: &StatusChannel,
) -> anyhow::Result<()> {
    let span = debug_span!("sync-event", %block);
    let _g = span.enter();

    // Blocks for which there's no client state. Some of the l1 blocks could have been skipped.
    let start_height = state
        .params
        .rollup()
        .genesis_l1_height
        .max(state.last_height + 1);
    let end_height = block.height();

    let mut potentually_skipped_blocks = state
        .storage
        .l1()
        .get_canonical_blockid_range(start_height, end_height)?
        .iter()
        .zip((start_height..end_height).into_iter())
        .map(|(blk_id, height)| L1BlockCommitment::new(height, *blk_id))
        .collect::<Vec<_>>();
    potentually_skipped_blocks.push(*block);

    debug!("trying sync event");

    for next_block in potentually_skipped_blocks.iter() {
        let mut tries = 0;
        let mut wait_dur = state.config.retry_base_dur;
        loop {
            tries += 1;

            // Check if client state is already present, and skip if so.
            let next_client_state = state
                .storage
                .client_state()
                ._get_state_blocking(*next_block)?;

            if let Some(cs) = next_client_state {
                state.update_bookeeping(next_block.height(), Arc::new(cs));
                continue;
            }

            // Handle the block for which there's no client state.
            let e = match handle_block(state, next_block) {
                Err(e) => e,
                Ok(v) => {
                    // Happy case, we want this to happen.
                    continue;
                }
            };

            // If we hit the try limit, abort.
            if tries > state.config.retry_cnt_max {
                error!(err = %e, %tries, "failed to exec sync event, hit tries limit, aborting");
                return Err(e);
            }

            // Sleep and increase the wait dur.
            error!(err = %e, %tries, "failed to exec sync event, retrying...");
            thread::sleep(wait_dur);
            wait_dur = state.config.compute_retry_backoff(wait_dur);

            if shutdown.should_shutdown() {
                warn!("received shutdown signal");
                break;
            }
        }
    }

    // Update status channel with the latest client state.
    status_channel.update_client_state(state.cur_state().as_ref().clone());
    trace!("completed sync event");

    debug!(%block, "processed OK");

    Ok(())
}

fn handle_block(state: &mut WorkerState, block: &L1BlockCommitment) -> anyhow::Result<()> {
    // Perform the main step of deciding what the output we're operating on.
    let outp = state.advance_consensus_state(block)?;

    // Apply the actions produced from the state transition before we publish
    // the new state, so that any database changes from them are available when
    // things listening for the new state observe it.
    for action in outp.actions() {
        apply_action(action.clone(), state)?;
    }

    // Store the outputs.
    let clstate = state
        .storage
        .client_state()
        ._put_update_blocking(block, outp.clone())?;

    // Set.
    state.update_bookeeping(block.height(), clstate);

    Ok(())
}

fn apply_action(action: SyncAction, state: &WorkerState) -> anyhow::Result<()> {
    let ckpt_db = state.storage.checkpoint();
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
