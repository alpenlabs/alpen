//! Fork choice manager. Used to talk to the EL and pick the new fork choice.

#![allow(unused)]

use std::{collections::VecDeque, sync::Arc};

use strata_chain_worker::{ChainWorkerHandle, WorkerError, WorkerResult};
use strata_chainexec::{validation_util, ChainExecutor, TipState};
use strata_chaintsn::transition::process_block;
use strata_common::retry::{
    policies::ExponentialBackoff, retry_with_backoff, DEFAULT_ENGINE_CALL_MAX_RETRIES,
};
use strata_db::{errors::DbError, traits::BlockStatus, types::CheckpointConfStatus};
use strata_eectl::{engine::ExecEngineCtl, errors::EngineError, messages::ExecPayloadData};
use strata_ol_chain_types::{validate_block_structure, L2BlockBundle, L2BlockId, L2Header};
use strata_primitives::{
    epoch::EpochCommitment, l1::L1BlockCommitment, l2::L2BlockCommitment, params::Params,
};
use strata_state::{
    batch::EpochSummary,
    chain_state::Chainstate,
    client_state::{CheckpointState, ClientState},
    prelude::*,
    state_op::{StateCache, WriteBatchEntry},
};
use strata_status::*;
use strata_storage::{L2BlockManager, NodeStorage};
use strata_tasks::ShutdownGuard;
use tokio::{
    runtime::Handle,
    sync::{mpsc, watch, Mutex},
};
use tracing::*;

use crate::{
    chain_worker_context::ChainWorkerCtx,
    csm::{ctl::CsmController, message::ForkChoiceMessage},
    errors::*,
    genesis::{check_needs_genesis, wait_for_genesis},
    tip_update::{compute_tip_update, TipUpdate},
    unfinalized_tracker::{self, UnfinalizedBlockTracker},
};

/// Tracks the parts of the chain that haven't been finalized on-chain yet.
#[expect(missing_debug_implementations)]
pub struct ForkChoiceManager {
    /// Consensus parameters.
    params: Arc<Params>,

    /// Common node storage interface.
    storage: Arc<NodeStorage>,

    /// Tracks unfinalized block tips.
    chain_tracker: unfinalized_tracker::UnfinalizedBlockTracker,

    /// Handle to the chain worker thread.
    chain_worker: Arc<ChainWorkerHandle>,

    /// Current best block.
    // TODO make sure we actually want to have this
    cur_best_block: L2BlockCommitment,

    /// Current toplevel chainstate we can do quick validity checks of new
    /// blocks against.
    cur_chainstate: Arc<Chainstate>,

    /// Epochs we know to be finalized from L1 checkpoints but whose corresponding
    /// L2 blocks we have not seen.
    epochs_pending_finalization: VecDeque<EpochCommitment>,
}

impl ForkChoiceManager {
    /// Constructs a new instance we can run the tracker with.
    pub fn new(
        params: Arc<Params>,
        storage: Arc<NodeStorage>,
        chain_tracker: unfinalized_tracker::UnfinalizedBlockTracker,
        chain_worker: Arc<ChainWorkerHandle>,
        cur_best_block: L2BlockCommitment,
        cur_chainstate: Arc<Chainstate>,
    ) -> Self {
        Self {
            params,
            storage,
            chain_tracker,
            chain_worker,
            cur_best_block,
            cur_chainstate,
            epochs_pending_finalization: VecDeque::new(),
        }
    }

    // TODO is this right?
    #[expect(unused)]
    fn finalized_tip(&self) -> &L2BlockId {
        self.chain_tracker.finalized_tip()
    }

    pub fn cur_best_block(&self) -> L2BlockCommitment {
        self.cur_best_block
    }

    fn set_block_status(&self, id: &L2BlockId, status: BlockStatus) -> Result<(), DbError> {
        self.storage.l2().set_block_status_blocking(id, status)?;
        Ok(())
    }

    fn get_block_status(&self, id: &L2BlockId) -> Result<Option<BlockStatus>, DbError> {
        self.storage.l2().get_block_status_blocking(id)
    }

    fn get_block_data(&self, id: &L2BlockId) -> Result<Option<L2BlockBundle>, DbError> {
        self.storage.l2().get_block_data_blocking(id)
    }

    fn get_block_slot(&self, blkid: &L2BlockId) -> anyhow::Result<u64> {
        // FIXME this is horrible but it makes our current use case much faster, see below
        if blkid == self.cur_best_block.blkid() {
            return Ok(self.cur_best_block.slot());
        }

        // FIXME we should have some in-memory cache of blkid->height, although now that we use the
        // manager this is less significant because we're cloning what's already in memory
        let block = self
            .get_block_data(blkid)?
            .ok_or(Error::MissingL2Block(*blkid))?;
        Ok(block.header().slot())
    }

    #[expect(unused)]
    fn get_block_chainstate(
        &self,
        block: &L2BlockCommitment,
    ) -> anyhow::Result<Option<Arc<Chainstate>>> {
        // If the chainstate we're looking for is the current chainstate, just
        // return that without taking the slow path.
        if block.blkid() == self.cur_best_block.blkid() {
            return Ok(Some(self.cur_chainstate.clone()));
        }

        Ok(self
            .storage
            .chainstate()
            .get_slot_write_batch_blocking(*block.blkid())?
            .map(|wb| Arc::new(wb.into_toplevel())))
    }

    /// Tries to execute the block, returning an error if applicable.
    fn try_exec_block(&mut self, block: &L2BlockCommitment) -> WorkerResult<()> {
        self.chain_worker.try_exec_block_blocking(*block)
    }

    /// Updates the stored current state.
    fn update_tip_block(
        &mut self,
        block: L2BlockCommitment,
        state: Arc<Chainstate>,
    ) -> WorkerResult<()> {
        self.cur_best_block = block;
        self.cur_chainstate = state;
        self.chain_worker.update_safe_tip_blocking(block)
    }

    fn attach_block(&mut self, blkid: &L2BlockId, bundle: &L2BlockBundle) -> anyhow::Result<bool> {
        let new_tip = self.chain_tracker.attach_block(*blkid, bundle.header())?;

        // maybe more logic here?

        Ok(new_tip)
    }

    /// Updates the bookkeeping to finalize and epoch.
    fn finalize_epoch(&mut self, epoch: &EpochCommitment) -> anyhow::Result<()> {
        // Safety check.
        if epoch.epoch() < self.get_chainstate_finalized_epoch().epoch() {
            return Err(
                Error::FinalizeOldEpoch(*epoch, *self.get_chainstate_finalized_epoch()).into(),
            );
        }

        // Do the leg work of applying the finalization.
        let tip_update = TipState::new(self.cur_best_block(), *epoch);
        self.chain_worker.finalize_epoch_blocking(*epoch)?;

        // Now update the in memory bookkeeping about it.
        self.chain_tracker.update_finalized_epoch(epoch)?;

        // Clear out old pending entries.
        self.clear_pending_epochs(epoch);

        Ok(())
    }

    fn get_chainstate_cur_epoch(&self) -> u64 {
        self.cur_chainstate.cur_epoch()
    }

    pub fn get_chainstate_prev_epoch(&self) -> &EpochCommitment {
        self.cur_chainstate.prev_epoch()
    }

    fn get_chainstate_finalized_epoch(&self) -> &EpochCommitment {
        self.cur_chainstate.finalized_epoch()
    }

    /// Gets the most recently finalized epoch, even if it's one that we haven't
    /// accepted as a new base yet due to missing intermediary blocks.
    fn get_most_recently_finalized_epoch(&self) -> &EpochCommitment {
        self.epochs_pending_finalization
            .back()
            .unwrap_or(self.chain_tracker.finalized_epoch())
    }

    /// Does handling to accept an epoch as finalized before we've actually validated it.
    fn attach_epoch_pending_finalization(&mut self, epoch: EpochCommitment) -> bool {
        let last_finalized_epoch = self.get_most_recently_finalized_epoch();

        if epoch.is_null() {
            warn!("tried to finalize null epoch");
            return false;
        }

        // Some checks to make sure we don't go backwards.
        if last_finalized_epoch.last_slot() > 0 {
            let epoch_advances = epoch.epoch() > last_finalized_epoch.epoch();
            let block_advances = epoch.last_slot() > last_finalized_epoch.last_slot();
            if !epoch_advances || !block_advances {
                warn!(?last_finalized_epoch, received = ?epoch, "received invalid or out of order epoch");
                return false;
            }
        }

        self.epochs_pending_finalization.push_back(epoch);

        true
    }

    fn find_latest_pending_finalizable_epoch(&self) -> Option<(usize, EpochCommitment)> {
        // the latest epoch which we have processed and is safe to finalize
        if self.cur_chainstate.prev_epoch().is_null() {
            return None;
        }
        let prev_epoch = self.cur_chainstate.prev_epoch().epoch();
        self.epochs_pending_finalization
            .iter()
            .enumerate()
            .rev()
            .find(|(_, epoch)| epoch.epoch() <= prev_epoch)
            .map(|(a, b)| (a, *b))
    }

    fn clear_pending_epochs(&mut self, cur_fin_epoch: &EpochCommitment) {
        while self
            .epochs_pending_finalization
            .front()
            .is_some_and(|e| e.epoch() <= cur_fin_epoch.epoch())
        {
            self.epochs_pending_finalization
                .pop_front()
                .expect("fcm: pop epochs_pending_finalization");
        }
    }
}

/// Creates the forkchoice manager state from a database and rollup params.
pub fn init_forkchoice_manager(
    storage: &Arc<NodeStorage>,
    params: &Arc<Params>,
    csm_finalized_epoch: Option<EpochCommitment>,
    genesis_blkid: L2BlockId,
    chain_worker: Arc<ChainWorkerHandle>,
) -> anyhow::Result<ForkChoiceManager> {
    info!("initialized fcm test");
    // Load data about the last finalized block so we can use that to initialize
    // the finalized tracker.

    // XXX right now we have to do some special casing for if we don't have an
    // initial checkpoint for the genesis epoch

    let latest_tip = storage.l2().get_tip_block_blocking()?;
    let latest_chainstate = storage
        .chainstate()
        .get_slot_write_batch_blocking(latest_tip)?
        .ok_or(DbError::MissingSlotWriteBatch(latest_tip))?
        .into_toplevel();

    let chainstate_last_epoch = latest_chainstate.prev_epoch();

    let csm_finalized_epoch =
        csm_finalized_epoch.unwrap_or_else(|| EpochCommitment::new(0, 0, genesis_blkid));

    // pick whatever is the earliest
    let finalized_epoch = if chainstate_last_epoch.epoch() < csm_finalized_epoch.epoch() {
        *chainstate_last_epoch
    } else {
        csm_finalized_epoch
    };

    debug!(?finalized_epoch, "loading from finalized block...");

    // Populate the unfinalized block tracker.
    let mut chain_tracker =
        unfinalized_tracker::UnfinalizedBlockTracker::new_empty(finalized_epoch);
    chain_tracker.load_unfinalized_blocks(storage.l2().as_ref())?;

    let cur_tip_block = determine_start_tip(&chain_tracker, storage.l2())?;
    debug!(?chain_tracker, "init chain tracker");

    // Load in that block's chainstate.
    let tip_blkid = *cur_tip_block.blkid();
    let chainstate = storage
        .chainstate()
        .get_slot_write_batch_blocking(tip_blkid)?
        .ok_or(DbError::MissingSlotWriteBatch(tip_blkid))?
        .into_toplevel();

    // Actually assemble the forkchoice manager state.
    let mut fcm = ForkChoiceManager::new(
        params.clone(),
        storage.clone(),
        chain_tracker,
        chain_worker,
        cur_tip_block,
        Arc::new(chainstate),
    );

    if finalized_epoch != csm_finalized_epoch {
        // csm is ahead of chainstate
        // search for all pending checkpoints
        for epoch in finalized_epoch.epoch()..=csm_finalized_epoch.epoch() {
            let Some(checkpoint_entry) = storage.checkpoint().get_checkpoint_blocking(epoch)?
            else {
                warn!(%epoch, "missing expected checkpoint entry");
                continue;
            };
            if let CheckpointConfStatus::Finalized(_) = checkpoint_entry.confirmation_status {
                let commitment = checkpoint_entry
                    .checkpoint
                    .batch_info()
                    .get_epoch_commitment();
                fcm.attach_epoch_pending_finalization(commitment);
            }
        }
    }

    Ok(fcm)
}

/// Determines the starting chain tip.  For now, this is just the block with the
/// highest index, choosing the lowest ordered blockid in the case of ties.
fn determine_start_tip(
    unfin: &UnfinalizedBlockTracker,
    l2_block_manager: &L2BlockManager,
) -> anyhow::Result<L2BlockCommitment> {
    let mut iter = unfin.chain_tips_iter();

    let mut best = iter.next().expect("fcm: no chain tips");
    let mut best_slot = l2_block_manager
        .get_block_data_blocking(best)?
        .ok_or(Error::MissingL2Block(*best))?
        .header()
        .slot();

    // Iterate through the remaining elements and choose.
    for blkid in iter {
        let blkid_slot = l2_block_manager
            .get_block_data_blocking(blkid)?
            .ok_or(Error::MissingL2Block(*best))?
            .header()
            .slot();

        if blkid_slot == best_slot && blkid < best {
            best = blkid;
        } else if blkid_slot > best_slot {
            best = blkid;
            best_slot = blkid_slot;
        }
    }

    Ok(L2BlockCommitment::new(best_slot, *best))
}

/// Main tracker task that takes a ready fork choice manager and some IO stuff.
#[allow(clippy::too_many_arguments)]
pub fn tracker_task(
    shutdown: ShutdownGuard,
    handle: Handle,
    storage: Arc<NodeStorage>,
    fcm_rx: mpsc::Receiver<ForkChoiceMessage>,
    chain_worker: Arc<ChainWorkerHandle>,
    params: Arc<Params>,
    status_channel: StatusChannel,
) -> anyhow::Result<()> {
    // TODO only print this if we *don't* have genesis yet, somehow
    info!("waiting for genesis before starting forkchoice logic");

    let genesis_block_id = handle.block_on(async {
        while storage
            .l2()
            .get_blocks_at_height_blocking(0)
            .unwrap()
            .is_empty()
        {
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        }
        storage
            .l2()
            .get_blocks_at_height_blocking(0)
            .unwrap()
            .first()
            .cloned()
            .expect("genesis should be in")
    });

    let (_, init_state) = storage.client_state().fetch_most_recent_state()?.unwrap();
    info!(?init_state, "starting forkchoice logic");

    // Now that we have the database state in order, we can actually init the
    // FCM.
    let mut fcm = match init_forkchoice_manager(
        &storage,
        &params,
        init_state.get_declared_final_epoch(),
        genesis_block_id,
        chain_worker,
    ) {
        Ok(fcm) => fcm,
        Err(e) => {
            error!(err = %e, "failed to init forkchoice manager!");
            return Err(e);
        }
    };

    let cur_tip = fcm.cur_best_block();
    let prev_epoch = *fcm.get_chainstate_prev_epoch();

    // Update status.
    // TODO: avoid repetition from process_fc_message
    let status = ChainSyncStatus {
        tip: fcm.cur_best_block,
        prev_epoch: *fcm.get_chainstate_prev_epoch(),
        finalized_epoch: *fcm.chain_tracker.finalized_epoch(),
        // FIXME this is a bit convoluted, could this be simpler?
        safe_l1: fcm.cur_chainstate.l1_view().get_safe_block(),
    };
    let update = ChainSyncStatusUpdate::new(status, fcm.cur_chainstate.clone());
    status_channel.update_chain_sync_status(update);

    handle_unprocessed_blocks(&mut fcm, &storage, &status_channel)?;

    if let Err(e) = forkchoice_manager_task_inner(&shutdown, handle, fcm, fcm_rx, status_channel) {
        error!(err = ?e, "tracker aborted");
        return Err(e);
    }

    Ok(())
}

/// Check if there are unprocessed L2 blocks in the database.
/// If there are, pass them to fcm.
pub fn handle_unprocessed_blocks(
    fcm: &mut ForkChoiceManager,
    storage: &NodeStorage,
    status_channel: &StatusChannel,
) -> anyhow::Result<()> {
    info!("checking for unprocessed L2 blocks");

    let l2_block_manager = storage.l2();
    let mut slot = fcm.cur_best_block.slot() + 1;
    loop {
        let blkids = l2_block_manager.get_blocks_at_height_blocking(slot)?;
        if blkids.is_empty() {
            break;
        }

        warn!(%slot, ?blkids, "found extra L2 blocks");
        for blockid in blkids {
            let status = l2_block_manager.get_block_status_blocking(&blockid)?;
            if let Some(BlockStatus::Invalid) = status {
                continue;
            }

            process_fc_message(ForkChoiceMessage::NewBlock(blockid), fcm, status_channel)?;
        }
        slot += 1;
    }

    info!("finished processing extra blocks");

    Ok(())
}

#[allow(clippy::large_enum_variant)]
enum FcmEvent {
    NewFcmMsg(ForkChoiceMessage),
    NewStateUpdate(ClientState),
    Abort,
}

pub fn forkchoice_manager_task_inner(
    shutdown: &ShutdownGuard,
    handle: Handle,
    mut fcm_state: ForkChoiceManager,
    mut fcm_rx: mpsc::Receiver<ForkChoiceMessage>,
    status_channel: StatusChannel,
) -> anyhow::Result<()> {
    let mut cl_rx = status_channel.subscribe_checkpoint_state();
    loop {
        // Check if we should shut down.
        if shutdown.should_shutdown() {
            warn!("fcm task received shutdown signal");
            break;
        }

        let fcm_ev = wait_for_fcm_event(&handle, &mut fcm_rx, &mut cl_rx);

        // Check again in case we got the signal while waiting.
        if shutdown.should_shutdown() {
            warn!("fcm task received shutdown signal");
            break;
        }

        match fcm_ev {
            FcmEvent::NewFcmMsg(m) => process_fc_message(m, &mut fcm_state, &status_channel),
            FcmEvent::NewStateUpdate(st) => handle_new_client_state(&mut fcm_state, st),
            FcmEvent::Abort => break,
        }?;
    }

    info!("FCM exiting");

    Ok(())
}

fn wait_for_fcm_event(
    handle: &Handle,
    fcm_rx: &mut mpsc::Receiver<ForkChoiceMessage>,
    cl_rx: &mut watch::Receiver<CheckpointState>,
) -> FcmEvent {
    handle.block_on(async {
        tokio::select! {
            m = fcm_rx.recv() => {
                m.map(FcmEvent::NewFcmMsg).unwrap_or_else(|| {
                    trace!("input channel closed");
                    FcmEvent::Abort
                })
            }
            c = wait_for_client_change(cl_rx) => {
                c.map(FcmEvent::NewStateUpdate).unwrap_or_else(|_| {
                    trace!("ClientState update channel closed");
                    FcmEvent::Abort
                })
            }
        }
    })
}

/// Waits until there's a new client state and returns the client state.
async fn wait_for_client_change(
    cl_rx: &mut watch::Receiver<CheckpointState>,
) -> Result<ClientState, watch::error::RecvError> {
    cl_rx.changed().await?;
    let state = cl_rx.borrow_and_update().clone();
    Ok(state.client_state)
}

fn process_fc_message(
    msg: ForkChoiceMessage,
    fcm_state: &mut ForkChoiceManager,
    status_channel: &StatusChannel,
) -> anyhow::Result<()> {
    match msg {
        ForkChoiceMessage::NewBlock(blkid) => {
            strata_common::check_bail_trigger("fcm_new_block");

            let block_bundle = fcm_state
                .get_block_data(&blkid)?
                .ok_or(Error::MissingL2Block(blkid))?;

            let slot = block_bundle.header().slot();
            info!(%slot, %blkid, "processing new block");

            let ok = match handle_new_block(fcm_state, &block_bundle) {
                Ok(v) => v,
                Err(e) => {
                    if let Some(EngineError::Other(_)) = e.downcast_ref() {
                        return Err(e);
                    }
                    // Really we shouldn't emit this error unless there's a
                    // problem checking the block in general and it could be
                    // valid or invalid, but we're kinda sloppy with errors
                    // here so let's try to avoid crashing the FCM task?
                    error!(%slot, %blkid, "error processing block, interpreting as invalid\n{e:?}");
                    false
                }
            };

            let status = if ok {
                // check if any pending blocks can be finalized
                if let Err(err) = handle_epoch_finalization(fcm_state) {
                    error!(%err, "failed to finalize epoch");
                    if let Some(EngineError::Other(_)) = err.downcast_ref() {
                        return Err(err);
                    }
                }

                // Update status.
                let status = ChainSyncStatus {
                    tip: fcm_state.cur_best_block,
                    prev_epoch: *fcm_state.get_chainstate_prev_epoch(),
                    finalized_epoch: *fcm_state.chain_tracker.finalized_epoch(),
                    // FIXME this is a bit convoluted, could this be simpler?
                    safe_l1: fcm_state.cur_chainstate.l1_view().get_safe_block(),
                };

                let update = ChainSyncStatusUpdate::new(status, fcm_state.cur_chainstate.clone());
                trace!(%blkid, "publishing new chainstate");
                status_channel.update_chain_sync_status(update);

                BlockStatus::Valid
            } else {
                // Emit invalid block warning.
                warn!(%blkid, "rejecting invalid block");
                BlockStatus::Invalid
            };

            fcm_state.set_block_status(&blkid, status)?;
        }
    }

    Ok(())
}

fn handle_new_block(
    fcm_state: &mut ForkChoiceManager,
    bundle: &L2BlockBundle,
) -> anyhow::Result<bool> {
    let slot = bundle.header().slot();
    let blkid = &bundle.header().get_blockid();
    info!(%blkid, %slot, "handling new block");

    /*
      debug!(?fcm_state.cur_best_block);
      debug!(?fcm_state.chain_tracker);
      debug!(?fcm_state.cur_chainstate);
    */

    // First, decide if the block seems correctly signed and we haven't
    // already marked it as invalid.
    let check_res = validation_util::check_block_proposal_valid(
        blkid,
        bundle.block(),
        fcm_state.params.rollup(),
    );
    if check_res.is_err() {
        // It's invalid, write that and return.
        return Ok(false);
    }

    // This stores the block output in the database, which lets us make queries
    // about it, at least until it gets reorged out by another block being
    // finalized.
    let bc = L2BlockCommitment::new(bundle.header().slot(), *blkid);
    let exec_ok = match fcm_state.try_exec_block(&bc) {
        Ok(()) => true,
        Err(err) => {
            // TODO Need some way to distinguish an invalid block from a exec failure
            error!(%err, "try_exec_block failed");
            false
        }
    };

    if exec_ok {
        fcm_state.set_block_status(blkid, BlockStatus::Valid)?;
    } else {
        fcm_state.set_block_status(blkid, BlockStatus::Invalid)?;
        return Ok(false);
    }

    // Insert block into pending block tracker and figure out if we
    // should switch to it as a potential head.  This returns if we
    // created a new tip instead of advancing an existing tip.
    let cur_tip = *fcm_state.cur_best_block.blkid();
    let new_tip = fcm_state.attach_block(blkid, bundle)?;
    if new_tip {
        debug!(?blkid, "created new branching tip");
    }

    // Now decide what the new tip should be and figure out how to get there.
    let best_block = pick_best_block(
        &cur_tip,
        fcm_state.chain_tracker.chain_tips_iter(),
        fcm_state.storage.l2(),
    )?;

    // TODO make configurable
    let depth = 100;

    let tip_update = compute_tip_update(&cur_tip, best_block, depth, &fcm_state.chain_tracker)?;
    let Some(tip_update) = tip_update else {
        // In this case there's no change.
        return Ok(true);
    };

    let tip_blkid = *tip_update.new_tip();
    debug!(%tip_blkid, "have new tip, applying update");

    // Apply the reorg.
    let res = match apply_tip_update(tip_update, fcm_state, bundle) {
        Ok(()) => {
            info!(%tip_blkid, "new chain tip");

            Ok(true)
        }

        Err(e) => {
            warn!(err = ?e, "failed to compute CL STF");

            // Specifically state transition errors we want to handle
            // specially so that we can remember to not accept the block again.
            if let Some(Error::InvalidStateTsn(inv_blkid, _)) = e.downcast_ref() {
                warn!(
                    ?blkid,
                    ?inv_blkid,
                    "invalid block on seemingly good fork, rejecting block"
                );

                Ok(false)
            } else {
                // Everything else we should fail on, signalling indeterminate
                // status for the block.
                Err(e)
            }
        }
    };

    debug!(?fcm_state.cur_best_block);
    debug!(?fcm_state.chain_tracker);
    debug!(?fcm_state.cur_chainstate);

    res
}

/// Returns if we should switch to the new fork.  This is dependent on our
/// current tip and any of the competing forks.  It's "sticky" in that it'll try
/// to stay where we currently are unless there's a definitely-better fork.
fn pick_best_block<'t>(
    cur_tip: &'t L2BlockId,
    tips_iter: impl Iterator<Item = &'t L2BlockId>,
    l2_block_manager: &L2BlockManager,
) -> Result<&'t L2BlockId, Error> {
    let mut best_tip = cur_tip;
    let mut best_block = l2_block_manager
        .get_block_data_blocking(best_tip)?
        .ok_or(Error::MissingL2Block(*best_tip))?;

    // The implementation of this will only switch to a new tip if it's a higher
    // height than our current tip.  We'll make this more sophisticated in the
    // future if we have a more sophisticated consensus protocol.
    for other_tip in tips_iter {
        if other_tip == cur_tip {
            continue;
        }

        let other_block = l2_block_manager
            .get_block_data_blocking(other_tip)?
            .ok_or(Error::MissingL2Block(*other_tip))?;

        let best_header = best_block.header();
        let other_header = other_block.header();

        if other_header.slot() > best_header.slot() {
            best_tip = other_tip;
            best_block = other_block;
        }
    }

    Ok(best_tip)
}

fn apply_tip_update(
    update: TipUpdate,
    fcm_state: &mut ForkChoiceManager,
    bundle: &L2BlockBundle,
) -> anyhow::Result<()> {
    match update {
        // Easy case.
        TipUpdate::ExtendTip(_cur, new) => {
            // Update the tip block in the FCM state.
            let chainstate = fcm_state
                .storage
                .chainstate()
                .get_slot_write_batch_blocking(new)?
                .ok_or(DbError::MissingSlotWriteBatch(new))?
                .into_toplevel();
            fcm_state.update_tip_block(
                bundle.block().header().get_block_commitment(),
                Arc::new(chainstate),
            )?;

            Ok(())
        }

        // Weird case that shouldn't normally happen.
        TipUpdate::LongExtend(_cur, mut intermediate, new) => {
            if intermediate.is_empty() {
                warn!("tip update is a LongExtend that should have been a ExtendTip");
            }

            // Push the new block onto the end and then use that list as the
            // blocks we're applying.
            intermediate.push(new);

            //apply_blocks(intermediate.into_iter(), fcm_state)
            Ok(())
        }

        TipUpdate::Reorg(reorg) => {
            // See if we need to roll back recent changes.
            let pivot_blkid = reorg.pivot();
            let pivot_slot = fcm_state.get_block_slot(pivot_blkid)?;
            let pivot_block = L2BlockCommitment::new(pivot_slot, *pivot_blkid);

            // We probably need to roll back to an earlier block and update our
            // in-memory state first.
            if pivot_slot < fcm_state.cur_best_block.slot() {
                debug!(%pivot_blkid, %pivot_slot, "rolling back chainstate");
                revert_chainstate_to_block(&pivot_block, fcm_state)?;
            } else {
                warn!("got a reorg that didn't roll back to an earlier pivot");
            }

            // Now actually apply the new blocks in order.  This handles all of
            // the normal logic involves in extending the chain.
            //apply_blocks(reorg.apply_iter().copied(), fcm_state)?;

            // TODO any cleanup?

            Ok(())
        }

        TipUpdate::Revert(_cur, new) => {
            let slot = fcm_state.get_block_slot(&new)?;
            let block = L2BlockCommitment::new(slot, new);
            revert_chainstate_to_block(&block, fcm_state)?;
            Ok(())
        }
    }
}

/// Safely reverts the in-memory chainstate to a particular block, then rolls
/// back the writes on-disk.
fn revert_chainstate_to_block(
    block: &L2BlockCommitment,
    fcm_state: &mut ForkChoiceManager,
) -> anyhow::Result<()> {
    // Fetch the old state from the database and store in memory.  This
    // is also how  we validate that we actually *can* revert to this
    // block.
    let blkid = *block.blkid();
    let new_state = fcm_state
        .storage
        .chainstate()
        .get_slot_write_batch_blocking(blkid)?
        .ok_or(Error::MissingBlockChainstate(blkid))?
        .into_toplevel();
    fcm_state.update_tip_block(*block, Arc::new(new_state));

    // FIXME: Rollback the writes on the database that we no longer need.

    Ok(())
}

/*
/// Applies one or more blocks, updating the FCM state and persisting write
/// batches to disk.  The block's parent must be the current tip in the FCM.
///
/// This is a batch operation to handle applying multiple blocks at once.
///
/// This may leave dirty write batches in the database, however the in-memory
/// state update is atomic and only changes if the database has been
/// successfully written to here.
fn apply_blocks(
    blkids: impl Iterator<Item = L2BlockId>,
    fcm_state: &mut ForkChoiceManager,
) -> anyhow::Result<()> {
    let rparams = fcm_state.params.rollup().clone();

    let mut cur_state = fcm_state.cur_chainstate.as_ref().clone();
    let mut updates = Vec::new();

    for blkid in blkids {
        // Load the previous block and its post-state.
        let bundle = fcm_state
            .get_block_data(&blkid)?
            .ok_or(Error::MissingL2Block(blkid))?;

        let slot = bundle.header().slot();
        let epoch = bundle.header().epoch();

        // Set up a new logging span for this block.
        let block_span = debug_span!("vfyblk", %slot, %epoch, %blkid);
        let _guard = block_span.enter();

        let header = bundle.header();
        let body = bundle.body();
        let block = L2BlockCommitment::new(slot, blkid);

        // Get the prev epoch to check if the epoch advanced, and the prev
        // epoch's terminal in case we need it.
        let pre_state_epoch_finishing = cur_state.is_epoch_finishing();
        let pre_state_epoch = cur_state.cur_epoch();

        // Compute the transition write batch, then compute the new state
        // locally and update our going state.
        let mut prestate_cache = StateCache::new(cur_state);
        debug!("processing block transition");
        process_block(&mut prestate_cache, header, body, &rparams)
            .map_err(|e| Error::InvalidStateTsn(blkid, e))?;
        let wb = prestate_cache.finalize();
        let post_state = wb.new_toplevel_state();

        let post_state_epoch = post_state.cur_epoch();

        // Sanity check for new epoch being either same or +1.
        assert!(
            (!pre_state_epoch_finishing && post_state_epoch == pre_state_epoch)
                || (pre_state_epoch_finishing && post_state_epoch == pre_state_epoch + 1),
            "fcm: nonsensical post-state epoch (pre={pre_state_epoch}, post={post_state_epoch})"
        );

        // Verify state root matches.
        let computed_sr = post_state.compute_state_root();
        if *header.state_root() != computed_sr {
            warn!(block_sr = %header.state_root(), %computed_sr, "state root mismatch");
            Err(Error::StaterootMismatch)?
        }

        // If we advanced the epoch then we have to finish it.
        let is_terminal = post_state.is_epoch_finishing();
        if is_terminal {
            handle_finish_epoch(&blkid, &bundle, post_state, fcm_state)?;
        }

        cur_state = post_state.clone();

        // After each application we update the fork choice tip data in case we fail
        // to apply an update.
        updates.push((block, wb));
    }

    // If there wasn't actually any updates, do nothing.
    if updates.is_empty() {
        return Ok(());
    }

    let last_block = updates.last().map(|(b, _)| *b).unwrap();

    // Apply all the write batches.
    let chsman = fcm_state.storage.chainstate();
    for (block, wb) in updates {
        chsman.put_write_batch_blocking(block.slot(), WriteBatchEntry::new(wb, *block.blkid()))?;
    }

    // Update the tip block in the FCM state.
    fcm_state.update_tip_block(last_block, Arc::new(cur_state));

    Ok(())
}

/// Takes the block and post-state and inserts database entries to reflect the
/// epoch being finished on-chain.
///
/// There's some bookkeeping here that's slightly weird since in the way it
/// works now, the last block of an epoch brings the post-state to the new
/// epoch.  So the epoch's final state actually has cur_epoch be the *next*
/// epoch.  And the index we assign to the summary here actually uses the "prev
/// epoch", since that's what the epoch in question is here.
///
/// This will be simplified if/when we out the per-block and per-epoch
/// processing into two separate stages.
fn handle_finish_epoch(
    blkid: &L2BlockId,
    bundle: &L2BlockBundle,
    post_state: &Chainstate,
    fcm_state: &mut ForkChoiceManager,
) -> anyhow::Result<()> {
    // Construct the various parts of the summary
    // NOTE: epoch update in chainstate happens at first slot of next epoch
    // this code runs at final slot of current epoch.
    let prev_epoch_idx = post_state.cur_epoch();

    let prev_terminal = post_state.prev_epoch().to_block_commitment();

    let slot = bundle.header().slot();
    let terminal = L2BlockCommitment::new(slot, *blkid);

    let l1seg = bundle.l1_segment();
    assert!(
        !l1seg.new_manifests().is_empty(),
        "fcm: epoch finished without L1 records"
    );
    let new_tip_height = l1seg.new_height();
    let new_tip_blkid = l1seg.new_tip_blkid().expect("fcm: missing l1seg final L1");
    let new_l1_block = L1BlockCommitment::new(new_tip_height, new_tip_blkid);

    let epoch_final_state = post_state.compute_state_root();

    // Actually construct and insert the epoch summary.
    let summary = EpochSummary::new(
        prev_epoch_idx,
        terminal,
        prev_terminal,
        new_l1_block,
        epoch_final_state,
    );

    // TODO convert to Display
    debug!(?summary, "finishing chain epoch");

    fcm_state
        .storage
        .checkpoint()
        .insert_epoch_summary_blocking(summary)?;

    Ok(())
}
*/

fn handle_new_client_state(
    fcm_state: &mut ForkChoiceManager,
    cs: ClientState,
) -> anyhow::Result<()> {
    let Some(new_fin_epoch) = cs.get_declared_final_epoch() else {
        debug!("got new CSM state, but finalized epoch still unset, ignoring");
        return Ok(());
    };

    info!(?new_fin_epoch, "got new finalized block");
    fcm_state.attach_epoch_pending_finalization(new_fin_epoch);

    match handle_epoch_finalization(fcm_state) {
        Err(err) => {
            error!(%err, "failed to finalize epoch");
            if let Some(EngineError::Other(_)) = err.downcast_ref() {
                return Err(err);
            }
        }
        Ok(Some(finalized_epoch)) if finalized_epoch == new_fin_epoch => {
            debug!(?finalized_epoch, "finalized latest epoch");
        }
        Ok(Some(finalized_epoch)) => {
            debug!(?finalized_epoch, "finalized earlier epoch");
        }
        Ok(None) => {
            // there were no epochs that could be finalized
            warn!("did not finalize epoch");
        }
    };

    Ok(())
}

/// Check if any pending epochs can be finalized.
/// If multiple are available, finalize the latest epoch that can be finalized.
/// Remove the finalized epoch and all earlier epochs from pending queue.
///
/// Note: Finalization in this context:
///     1. Update chaintip tracker's base block
///     2. Message execution engine to mark block corresponding to last block of this epoch as
///        finalized in the EE.
///
/// Return commitment to epoch that was finalized, if any.
fn handle_epoch_finalization(
    fcm_state: &mut ForkChoiceManager,
) -> anyhow::Result<Option<EpochCommitment>> {
    let Some((idx, next_finalizable_epoch)) = fcm_state.find_latest_pending_finalizable_epoch()
    else {
        // no new blocks to finalize
        return Ok(None);
    };

    debug!(
        ?next_finalizable_epoch,
        ?idx,
        last_epoch = ?fcm_state.cur_chainstate.prev_epoch(),
        "got new CSM state, updating finalized block"
    );

    fcm_state.finalize_epoch(&next_finalizable_epoch)?;

    info!(?next_finalizable_epoch, "updated finalized tip");
    //trace!(?fin_report, "finalization report");
    // TODO do something with the finalization report?

    Ok(Some(next_finalizable_epoch))
}
