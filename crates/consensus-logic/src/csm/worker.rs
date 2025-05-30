//! Consensus logic worker task.

use std::{sync::Arc, thread};

use strata_db::types::{CheckpointConfStatus, CheckpointEntry, CheckpointProvingStatus};
use strata_eectl::engine::ExecEngineCtl;
use strata_primitives::prelude::*;
use strata_state::{
    client_state::{ClientState, ClientStateMut},
    csm_status::CsmStatus,
    operation::{ClientUpdateOutput, SyncAction},
    sync_event::SyncEvent,
};
use strata_status::StatusChannel;
use strata_storage::{CheckpointDbManager, NodeStorage};
use strata_tasks::ShutdownGuard;
use tokio::{
    sync::{broadcast, mpsc},
    time,
};
use tracing::*;

use super::{
    client_transition,
    config::CsmExecConfig,
    message::{ClientUpdateNotif, CsmMessage},
};
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

    /// Checkpoint manager.
    checkpoint_manager: Arc<CheckpointDbManager>,

    /// Current state index.
    cur_state_idx: u64,

    /// Current state ref.
    cur_state: Arc<ClientState>,

    /// Broadcast channel used to publish state updates.
    cupdate_tx: broadcast::Sender<Arc<ClientUpdateNotif>>,
}

impl WorkerState {
    /// Constructs a new instance by reconstructing the current consensus state
    /// from the provided database layer.
    pub fn open(
        params: Arc<Params>,
        storage: Arc<NodeStorage>,
        cupdate_tx: broadcast::Sender<Arc<ClientUpdateNotif>>,
        checkpoint_manager: Arc<CheckpointDbManager>,
    ) -> anyhow::Result<Self> {
        let csman = storage.client_state();

        let (cur_state_idx, cur_state) = csman
            .get_most_recent_state_blocking()
            .ok_or(Error::MissingClientState(0))?;

        // TODO make configurable
        let config = CsmExecConfig {
            retry_base_dur: time::Duration::from_millis(1000),
            // These settings makes the last retry delay be 6 seconds.
            retry_cnt_max: 20,
            retry_backoff_mult: 1120,
        };

        Ok(Self {
            params,
            config,
            storage,
            cur_state_idx,
            cur_state,
            cupdate_tx,
            checkpoint_manager,
        })
    }

    /// Gets the index of the current state.
    pub const fn cur_event_idx(&self) -> u64 {
        self.cur_state_idx
    }

    /// Gets a ref to the consensus state from the inner state tracker.
    pub const fn cur_state(&self) -> &Arc<ClientState> {
        &self.cur_state
    }

    /// Gets a reference to checkpoint manager
    pub fn checkpoint_db(&self) -> &CheckpointDbManager {
        self.checkpoint_manager.as_ref()
    }

    fn get_sync_event(&self, idx: u64) -> anyhow::Result<Option<SyncEvent>> {
        Ok(self.storage.sync_event().get_sync_event_blocking(idx)?)
    }

    /// Fetches a sync event from storage.
    fn get_sync_event_ok(&self, idx: u64) -> anyhow::Result<SyncEvent> {
        Ok(self
            .get_sync_event(idx)?
            .ok_or(Error::MissingSyncEvent(idx))?)
    }

    /// Given the next event index, computes the state application if the
    /// requisite data is available.  Returns the output and the new state.
    ///
    /// This is copied from the old `StateTracker` type which we removed to
    /// simplify things.
    // TODO maybe remove output return value
    pub fn advance_consensus_state(&mut self, ev_idx: u64) -> anyhow::Result<ClientUpdateOutput> {
        let prev_ev_idx = ev_idx - 1;
        if prev_ev_idx != self.cur_state_idx {
            return Err(Error::SkippedEventIdx(prev_ev_idx, self.cur_state_idx).into());
        }

        // Load the event from the database.
        let ev = self.get_sync_event_ok(ev_idx)?;

        debug!(%ev_idx, ?ev, "processing sync event");

        // Compute the state transition.
        let context = client_transition::StorageEventContext::new(&self.storage);
        let mut state_mut = ClientStateMut::new(self.cur_state.as_ref().clone());
        client_transition::process_event(&mut state_mut, &ev, &context, &self.params)?;

        // Clone the state and apply the operations to it.
        let outp = state_mut.into_update();

        Ok(outp)
    }

    fn update_bookeeping(&mut self, ev_idx: u64, state: Arc<ClientState>) {
        debug!(%ev_idx, ?state, "computed new consensus state");
        self.cur_state = state;
        self.cur_state_idx = ev_idx;
    }
}

/// Receives messages from channel to update consensus state with.
// TODO consolidate all these channels into container/"io" types
pub fn client_worker_task<E: ExecEngineCtl>(
    shutdown: ShutdownGuard,
    mut state: WorkerState,
    engine: Arc<E>,
    mut msg_rx: mpsc::Receiver<CsmMessage>,
    status_channel: StatusChannel,
) -> anyhow::Result<()> {
    info!("started CSM worker");

    while let Some(msg) = msg_rx.blocking_recv() {
        if let Err(e) = process_msg(
            &mut state,
            engine.as_ref(),
            &msg,
            &status_channel,
            &shutdown,
        ) {
            error!(err = %e, ?msg, "failed to process sync message, aborting!");
            break;
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
    engine: &impl ExecEngineCtl,
    msg: &CsmMessage,
    status_channel: &StatusChannel,
    shutdown: &ShutdownGuard,
) -> anyhow::Result<()> {
    match msg {
        CsmMessage::EventInput(idx) => {
            strata_common::check_bail_trigger("sync_event");

            // If we somehow missed a sync event we need to try to rerun those,
            // just in case.
            let cur_ev_idx = state.cur_event_idx();
            let next_exp_idx = cur_ev_idx + 1;
            for ev_idx in next_exp_idx..=*idx {
                if ev_idx < *idx {
                    warn!(%ev_idx, "Applying missed sync event.");
                }

                // FIXME: We should be explicit about what errors to retry instead of just
                // retrying whenever this fails.
                handle_sync_event_with_retry(state, engine, ev_idx, status_channel, shutdown)?;

                // Broadcast notifications and other stuffs
                post_handle_sync_event_hook(
                    ev_idx,
                    state.cur_state().clone(),
                    state,
                    status_channel,
                )?;
            }

            Ok(())
        }
    }
}

/// Repeatedly calls `handle_sync_event`, retrying on failure, up to a limit
/// after which we return with the most recent error.
fn handle_sync_event_with_retry(
    state: &mut WorkerState,
    engine: &impl ExecEngineCtl,
    ev_idx: u64,
    status_channel: &StatusChannel,
    shutdown: &ShutdownGuard,
) -> anyhow::Result<()> {
    // Fetch the sync event we're looking at.
    let Some(ev) = state.get_sync_event(ev_idx)? else {
        error!(%ev_idx, "tried to process missing sync event, aborting handle_sync_event!");
        return Ok(());
    };

    let span = debug_span!("sync-event", %ev_idx, %ev);
    let _g = span.enter();

    let mut tries = 0;
    let mut wait_dur = state.config.retry_base_dur;

    loop {
        tries += 1;

        // TODO demote to trace after we figure out the current issues
        debug!("trying sync event");

        let e = match handle_sync_event(state, engine, ev_idx, status_channel) {
            Err(e) => e,
            Ok(v) => {
                // Happy case, we want this to happen.
                trace!("completed sync event");
                return Ok(v);
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

    debug!(%ev_idx, %ev, "processed OK");

    Ok(())
}

fn handle_sync_event(
    state: &mut WorkerState,
    engine: &impl ExecEngineCtl,
    ev_idx: u64,
    status_channel: &StatusChannel,
) -> anyhow::Result<()> {
    // Perform the main step of deciding what the output we're operating on.
    let outp = state.advance_consensus_state(ev_idx)?;

    // Apply the actions produced from the state transition before we publish
    // the new state, so that any database changes from them are available when
    // things listening for the new state observe it.
    for action in outp.actions() {
        apply_action(action.clone(), state, engine, status_channel)?;
    }

    // Store the outputs.
    let clstate = state
        .storage
        .client_state()
        .put_update_blocking(ev_idx, outp.clone())?;

    // Now update worker state bookkeeping
    state.update_bookeeping(ev_idx, clstate);

    // Make sure that the new state index is set as expected.
    assert_eq!(state.cur_event_idx(), ev_idx);

    Ok(())
}

/// Apply send Update notification to listeners.
fn post_handle_sync_event_hook(
    ev_idx: u64,
    new_state: Arc<ClientState>,
    state: &WorkerState,
    status_channel: &StatusChannel,
) -> anyhow::Result<()> {
    // FIXME clean this up and make them take Arcs
    let mut status = CsmStatus::default();
    status.set_last_sync_ev_idx(ev_idx);
    status.update_from_client_state(new_state.as_ref());
    status_channel.update_client_state(new_state.as_ref().clone());

    trace!(%ev_idx, "sending client update notif");
    let update = ClientUpdateNotif::new(ev_idx, new_state);
    if state.cupdate_tx.send(Arc::new(update)).is_err() {
        // Is this actually useful?  Does this just error if there's no
        // listeners?
        warn!("failed to send broadcast for new CSM update");
    }
    Ok(())
}

fn apply_action(
    action: SyncAction,
    state: &WorkerState,
    _engine: &impl ExecEngineCtl,
    _status_channel: &StatusChannel,
) -> anyhow::Result<()> {
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

        SyncAction::L2Genesis(l1blkid) => {
            info!(%l1blkid, "locking in genesis!");

            // TODO: use l1blkid during chain state genesis ?

            // Save the genesis chainstate and block.
            let _chstate = genesis::init_genesis_chainstate(&state.params, &state.storage)
                .map_err(|err| {
                    error!(err = %err, "failed to compute chain genesis");
                    Error::GenesisFailed(err.to_string())
                })?;

            // TODO do we have to do anything here?
        }
    }

    Ok(())
}
