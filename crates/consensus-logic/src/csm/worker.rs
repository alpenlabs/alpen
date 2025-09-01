//! Consensus logic worker task.

use std::{sync::Arc, thread};

use strata_db::types::{CheckpointConfStatus, CheckpointEntry, CheckpointProvingStatus};
use strata_primitives::prelude::*;
use strata_state::{
    client_state::ClientState,
    operation::{ClientUpdateOutput, SyncAction},
};
use strata_status::StatusChannel;
use strata_storage::NodeStorage;
use strata_tasks::ShutdownGuard;
use tokio::{sync::mpsc, time};
use tracing::*;

use super::config::CsmExecConfig;
use crate::csm::client_transition::{self, EventContext, StorageEventContext};

/// Mutable worker state that we modify in the consensus worker task.
///
/// Unable to be shared across threads.  Any data we want to export we'll do
/// through another handle.
#[expect(missing_debug_implementations)]
pub struct WorkerState {
    /// Consensus parameters.
    params: Arc<Params>,

    /// Node storage handle.
    storage: Arc<NodeStorage>,

    /// Current state ref.
    cur_state: Arc<ClientState>,

    /// Current block that corresponds to cur_state
    cur_block: L1BlockCommitment,
}

impl WorkerState {
    /// Constructs a new instance by reconstructing the current consensus state
    /// from the provided database layer.
    pub fn open(params: Arc<Params>, storage: Arc<NodeStorage>) -> anyhow::Result<Self> {
        let (cur_block, cur_state) = storage
            .client_state()
            .fetch_most_recent_state()?
            .expect("missing initial client state?");

        Ok(Self {
            params,
            storage,
            cur_state: Arc::new(cur_state),
            cur_block,
        })
    }

    /// Gets a ref to the consensus state from the inner state tracker.
    pub fn cur_state(&self) -> &Arc<ClientState> {
        &self.cur_state
    }

    pub fn cur_block(&self) -> L1BlockCommitment {
        self.cur_block
    }

    /// Given the next l1 block, does the state transition, returning next [`ClientState`].
    pub fn advance_consensus_state(
        &self,
        next_block: &L1BlockManifest,
    ) -> anyhow::Result<(ClientState, Vec<SyncAction>)> {
        let id = next_block.blkid();
        debug!(%id, "processing l1 block");

        let context = client_transition::StorageEventContext::new(&self.storage);

        Ok(client_transition::transition_client_state(
            self.cur_state.as_ref().clone(),
            &self.cur_block,
            next_block,
            &context,
            &self.params,
        )?)
    }

    fn update_bookeeping(&mut self, block: L1BlockCommitment, state: Arc<ClientState>) {
        debug!(%block, ?state, "computed new consensus state");
        self.cur_state = state;
        self.cur_block = block;
    }
}

/// Receives l1 blocks from channel to update consensus state with.
// TODO consolidate all these channels into container/"io" types
pub fn client_worker_task(
    shutdown: ShutdownGuard,
    mut state: WorkerState,
    mut block_rx: mpsc::Receiver<L1BlockCommitment>,
    status_channel: StatusChannel,
) -> anyhow::Result<()> {
    info!("started CSM worker");

    // TODO make configurable
    let config = CsmExecConfig {
        retry_base_dur: time::Duration::from_millis(1000),
        // These settings makes the last retry delay be 6 seconds.
        retry_cnt_max: 20,
        retry_backoff_mult: 1120,
    };

    while let Some(msg) = block_rx.blocking_recv() {
        if let Err(e) =
            process_block_with_retries(&mut state, &msg, &status_channel, &config, &shutdown)
        {
            error!(err = %e, ?msg, "failed to process the block, aborting!");
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

fn process_block_with_retries(
    state: &mut WorkerState,
    incoming_block: &L1BlockCommitment,
    status_channel: &StatusChannel,
    config: &CsmExecConfig,
    shutdown: &ShutdownGuard,
) -> anyhow::Result<()> {
    strata_common::check_bail_trigger("sync_event");

    let span = debug_span!("csm-process-block", %incoming_block);
    let _g = span.enter();

    let mut skipped_blocks: Vec<_> = vec![];

    // Handle pre-genesis: if the block is before genesis we don't care about it.
    let genesis_trigger = state.params.rollup().genesis_l1_view.height();
    let height = incoming_block.height();
    if incoming_block.height() < genesis_trigger {
        #[cfg(test)]
        eprintln!(
                    "early L1 block at h={height} (gt={genesis_trigger}) you may have set up the test env wrong"
                );

        warn!(%height, "ignoring unexpected L1Block event before horizon");
        return Ok(());
    }

    // Traverse back the chain of l1 blocks until we find an l1 block which has ClientState.
    // Remember all the blocks along the way and pass it (in the reverse order) to process.
    let (pivot_block, pivot_state) = {
        let ctx = StorageEventContext::new(&state.storage);
        let mut cur_block = *incoming_block;
        let mut cur_state = ctx.get_client_state(&cur_block);

        while cur_state.is_err()
            && cur_block.height() >= state.params.rollup().genesis_l1_view.height()
        {
            let cur_block_mf = ctx.get_l1_block_manifest(cur_block.blkid())?;
            let prev_block_id = cur_block_mf.get_prev_blockid();

            // Push the manifest that corresponds to the current (not processed) block.
            skipped_blocks.push(cur_block_mf);

            // Set the cur block and state to point at the parent's block.
            cur_block = L1BlockCommitment::new(cur_block.height() - 1, prev_block_id);
            cur_state = ctx.get_client_state(&cur_block);
        }

        if cur_block.height() < state.params.rollup().genesis_l1_view.height() {
            // we reached the height before genesis (while traversing the tree of ClientStates),
            // for such a case there shouldn't be any ClientState besides the default one.
            (Default::default(), Default::default())
        } else {
            (cur_block, cur_state.unwrap())
        }
    };

    // Here pivot_block and pivot_state denote the first "parent" block that has ClientState
    // or the default one if nothing was found during traversing.
    // P.S. default block and state are actually present in the db as a valid pre-genesis state.
    state.update_bookeeping(pivot_block, Arc::new(pivot_state));

    // An "expected" length of the skipped_blocks is 1 (given no reorgs and no blocks skipped),
    // so log some information for other cases.
    if skipped_blocks.is_empty() {
        // At least incoming_block is expected to be present in the vec.
        warn!(%incoming_block, "somehow the client state already present for the block");
    } else if skipped_blocks.len() > 1 {
        info!(
            "CSM must handle additional parent blocks that were skipped, cnt: {:?}",
            skipped_blocks.len() - 1,
        );
    }

    // Traverse the whole unprocessed chain, starting from older blocks till incoming_block.
    for next_block in skipped_blocks.iter().rev() {
        process_block(state, next_block, config, shutdown)?;
        status_channel.update_client_state(state.cur_state().as_ref().clone(), state.cur_block());
    }

    debug!(%incoming_block, "processed OK");

    Ok(())
}

fn process_block(
    state: &mut WorkerState,
    block: &L1BlockManifest,
    config: &CsmExecConfig,
    shutdown: &ShutdownGuard,
) -> anyhow::Result<()> {
    debug!("trying to process the block");
    let mut tries = 0;
    let mut wait_dur = config.retry_base_dur;
    loop {
        tries += 1;

        // Handle the block for which there's no client state.
        let e = match handle_block(state, block) {
            Err(e) => e,
            Ok(_) => {
                // Happy case, we want this to happen.
                return Ok(());
            }
        };

        // If we hit the try limit, abort.
        if tries > config.retry_cnt_max {
            error!(err = %e, %tries, "failed to exec sync event, hit tries limit, aborting");
            return Err(e);
        }

        // Sleep and increase the wait dur.
        error!(err = %e, %tries, "failed to exec sync event, retrying...");
        thread::sleep(wait_dur);
        wait_dur = config.compute_retry_backoff(wait_dur);

        if shutdown.should_shutdown() {
            warn!("received shutdown signal");
            break;
        }
    }
    Ok(())
}

fn handle_block(state: &mut WorkerState, block: &L1BlockManifest) -> anyhow::Result<()> {
    let block_id: L1BlockCommitment = block.into();
    // Perform the main step of deciding what the output we're operating on.
    let (next_state, actions) = state.advance_consensus_state(block)?;

    // Apply the actions produced from the state transition before we publish
    // the new state, so that any database changes from them are available when
    // things listening for the new state observe it.
    for action in actions.iter() {
        apply_action(action.clone(), &state.storage)?;
    }

    // Store the outputs.
    let clstate = state.storage.client_state().put_update_blocking(
        &block_id,
        ClientUpdateOutput::new(next_state, actions).clone(),
    )?;

    // Set.
    state.update_bookeeping(block_id, clstate);

    Ok(())
}

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
