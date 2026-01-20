//! Fork choice manager. Used to pick the new fork choice.

use std::{collections::VecDeque, sync::Arc};

use anyhow::anyhow;
use bitcoin::absolute::Height;
use strata_chain_worker_new::{ChainWorkerHandle, WorkerResult};
use strata_csm_types::{CheckpointState, ClientState};
use strata_csm_worker::CsmWorkerStatus;
use strata_db_types::{errors::DbError, traits::BlockStatus, types::CheckpointConfStatus};
use strata_eectl::errors::EngineError;
use strata_identifiers::Epoch;
use strata_ledger_types::IStateAccessor;
use strata_ol_chain_types_new::OLBlock;
use strata_ol_state_types::OLState;
use strata_params::{Params, RollupParams};
use strata_primitives::{
    crypto::verify_schnorr_sig, epoch::EpochCommitment, l2::L2BlockCommitment, Buf32, CredRule,
    L1BlockCommitment, L2BlockId, OLBlockId,
};
use strata_service::ServiceMonitor;
use strata_status::*;
use strata_storage::{NodeStorage, OLBlockManager};
use strata_tasks::ShutdownGuard;
use tokio::{
    runtime::Handle,
    sync::{mpsc, watch},
    time::{self, sleep},
};
use tracing::*;

use crate::{
    errors::*,
    message::ForkChoiceMessage,
    tip_update::{compute_tip_update, TipUpdate},
    unfinalized_tracker::{self, UnfinalizedBlockTracker},
};

/// Tracks the parts of the chain that haven't been finalized on-chain yet.
#[expect(
    missing_debug_implementations,
    reason = "Some inner types don't have Debug impls"
)]
pub struct ForkChoiceManager {
    /// Consensus parameters.
    params: Arc<Params>,

    /// Common node storage interface.
    storage: Arc<NodeStorage>,

    /// Tracks unfinalized block tips.
    chain_tracker: UnfinalizedBlockTracker,

    /// Handle to the chain worker thread.
    chain_worker: Arc<ChainWorkerHandle>,

    /// Current best block.
    // TODO make sure we actually want to have this
    cur_best_block: L2BlockCommitment,

    /// Current toplevel ol_state we can do quick validity checks of new
    /// blocks against.
    cur_olstate: Arc<OLState>,

    /// Epochs we know to be finalized from L1 checkpoints but whose corresponding
    /// OL blocks we have not seen.
    epochs_pending_finalization: VecDeque<EpochCommitment>,

    /// Csm Monitor
    csm_monitor: Arc<ServiceMonitor<CsmWorkerStatus>>,
}

impl ForkChoiceManager {
    /// Constructs a new instance we can run the tracker with.
    pub fn new(
        params: Arc<Params>,
        storage: Arc<NodeStorage>,
        chain_tracker: unfinalized_tracker::UnfinalizedBlockTracker,
        chain_worker: Arc<ChainWorkerHandle>,
        cur_best_block: L2BlockCommitment,
        cur_ol_state: Arc<OLState>,
        csm_monitor: Arc<ServiceMonitor<CsmWorkerStatus>>,
    ) -> Self {
        Self {
            params,
            storage,
            chain_tracker,
            chain_worker,
            cur_best_block,
            cur_olstate: cur_ol_state,
            epochs_pending_finalization: VecDeque::new(),
            csm_monitor,
        }
    }

    // TODO is this right?
    #[expect(unused, reason = "used for fork choice manager")]
    fn finalized_tip(&self) -> &OLBlockId {
        self.chain_tracker.finalized_tip()
    }

    fn set_block_status(&self, id: &OLBlockId, status: BlockStatus) -> Result<(), DbError> {
        self.storage
            .ol_block()
            .set_block_status_blocking(*id, status)?;
        Ok(())
    }

    fn get_block_data(&self, id: &OLBlockId) -> Result<Option<OLBlock>, DbError> {
        self.storage.ol_block().get_block_data_blocking(*id)
    }

    fn get_block_slot(&self, blkid: &OLBlockId) -> anyhow::Result<u64> {
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

    #[expect(unused, reason = "used for fork choice manager")]
    fn get_block_ol_state(
        &self,
        block: &L2BlockCommitment,
    ) -> anyhow::Result<Option<Arc<OLState>>> {
        // If the ol_state we're looking for is the current ol_state, just
        // return that without taking the slow path.
        if block.blkid() == self.cur_best_block.blkid() {
            return Ok(Some(self.cur_olstate.clone()));
        }

        Ok(self
            .storage
            .ol_state()
            .get_toplevel_ol_state_blocking(*block)?)
    }

    /// Tries to execute the block, returning an error if applicable.
    fn try_exec_block(&mut self, block: &L2BlockCommitment) -> WorkerResult<()> {
        self.chain_worker.try_exec_block_blocking(*block)
    }

    /// Updates the stored current state.
    fn update_tip_block(
        &mut self,
        block: L2BlockCommitment,
        state: Arc<OLState>,
    ) -> WorkerResult<()> {
        self.cur_best_block = block;
        self.cur_olstate = state;
        self.chain_worker.update_safe_tip_blocking(block)
    }

    fn attach_block(&mut self, blkid: &OLBlockId, bundle: &OLBlock) -> anyhow::Result<bool> {
        let new_tip = self
            .chain_tracker
            .attach_block(*blkid, bundle.signed_header())?;

        // maybe more logic here?

        Ok(new_tip)
    }

    /// Updates the bookkeeping to finalize and epoch.
    fn finalize_epoch(&mut self, epoch: &EpochCommitment) -> anyhow::Result<()> {
        // Safety check.
        let fin_epoch = self
            .csm_monitor
            .get_current()
            .last_finalized_epoch
            .unwrap_or(EpochCommitment::null());
        if epoch.epoch() < fin_epoch.epoch() {
            return Err(Error::FinalizeOldEpoch(*epoch, fin_epoch).into());
        }

        // Do the leg work of applying the finalization.
        self.chain_worker.finalize_epoch_blocking(*epoch)?;

        // Now update the in memory bookkeeping about it.
        self.chain_tracker.update_finalized_epoch(epoch)?;

        // Clear out old pending entries.
        self.clear_pending_epochs(epoch);

        Ok(())
    }

    #[expect(unused, reason = "used for fork choice manager")]
    fn get_ol_state_cur_epoch(&self) -> Epoch {
        self.cur_olstate.cur_epoch()
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
        // If prev epoch is null return None
        let prev_epoch = self.cur_olstate.cur_epoch().saturating_sub(1);
        if prev_epoch == 0 {
            return None;
        }
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

    fn get_ol_state_prev_epoch(&self) -> EpochCommitment {
        *self.cur_olstate.previous_epoch()
    }
}

/// Creates the forkchoice manager state from a database and rollup params.
pub fn init_forkchoice_manager(
    storage: &Arc<NodeStorage>,
    params: &Arc<Params>,
    csm_finalized_epoch: Option<EpochCommitment>,
    genesis_blkid: OLBlockId,
    chain_worker: Arc<ChainWorkerHandle>,
    csm_monitor: Arc<ServiceMonitor<CsmWorkerStatus>>,
) -> anyhow::Result<ForkChoiceManager> {
    info!("initialized fcm test");
    // Load data about the last finalized block so we can use that to initialize
    // the finalized tracker.

    let csm_finalized_epoch =
        csm_finalized_epoch.unwrap_or_else(|| EpochCommitment::new(0, 0, genesis_blkid));

    // pick whatever is the earliest
    let finalized_epoch = csm_finalized_epoch; // TODO: correctly calculate finalized epoch

    debug!(?finalized_epoch, "loading from finalized block...");

    // Populate the unfinalized block tracker.
    let mut chain_tracker = UnfinalizedBlockTracker::new_empty(finalized_epoch);
    chain_tracker.load_unfinalized_ol_blocks(storage.ol_block().as_ref())?;

    let cur_tip_block = determine_start_tip(&chain_tracker, storage.ol_block())?;
    debug!(?chain_tracker, "init chain tracker");

    // Load in that block's ol_state.
    let tip_blkid = cur_tip_block;
    let ol_state = storage
        .ol_state()
        .get_toplevel_ol_state_blocking(tip_blkid)?
        .ok_or(DbError::MissingSlotWriteBatch(*tip_blkid.blkid()))?;

    // Actually assemble the forkchoice manager state.
    let mut fcm = ForkChoiceManager::new(
        params.clone(),
        storage.clone(),
        chain_tracker,
        chain_worker,
        cur_tip_block,
        ol_state,
        csm_monitor,
    );

    if finalized_epoch != csm_finalized_epoch {
        // csm is ahead of ol_state
        // search for all pending checkpoints
        for epoch in finalized_epoch.epoch()..=csm_finalized_epoch.epoch() {
            let Some(checkpoint_entry) =
                // TODO: use new checkpoint type and db(to be done in another ticket)
                storage.checkpoint().get_checkpoint_blocking(epoch as u64)?
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
    ol_block_mgr: &OLBlockManager,
) -> anyhow::Result<L2BlockCommitment> {
    let mut iter = unfin.chain_tips_iter();

    let mut best = iter.next().expect("fcm: no chain tips");
    let mut best_slot = ol_block_mgr
        .get_block_data_blocking(*best)?
        .ok_or(Error::MissingL2Block(*best))?
        .header()
        .slot();

    // Iterate through the remaining elements and choose.
    for blkid in iter {
        let blkid_slot = ol_block_mgr
            .get_block_data_blocking(*blkid)?
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
#[expect(clippy::too_many_arguments, reason = "this needs too many args")]
pub fn tracker_task(
    shutdown: ShutdownGuard,
    handle: Handle,
    storage: Arc<NodeStorage>,
    fcm_rx: mpsc::Receiver<ForkChoiceMessage>,
    chain_worker: Arc<ChainWorkerHandle>,
    params: Arc<Params>,
    status_channel: StatusChannel,
    csm_monitor: Arc<ServiceMonitor<CsmWorkerStatus>>,
) -> anyhow::Result<()> {
    // TODO only print this if we *don't* have genesis yet, somehow
    info!("waiting for genesis before starting forkchoice logic");

    let genesis_block_id = handle.block_on(async {
        while storage
            .ol_block()
            .get_blocks_at_height_blocking(0)
            .unwrap()
            .is_empty()
        {
            sleep(time::Duration::from_secs(1)).await;
        }
        storage
            .ol_block()
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
        csm_monitor,
    ) {
        Ok(fcm) => fcm,
        Err(e) => {
            error!(err = %e, "failed to init forkchoice manager!");
            return Err(e);
        }
    };

    // Update status.
    let last_l1_blk = L1BlockCommitment::new(
        Height::from_consensus(fcm.cur_olstate.last_l1_height()).expect("Invalid l1 height"),
        *fcm.cur_olstate.last_l1_blkid(),
    );
    let status = ChainSyncStatus {
        tip: fcm.cur_best_block,
        prev_epoch: fcm.get_ol_state_prev_epoch(),
        finalized_epoch: *fcm.chain_tracker.finalized_epoch(),
        safe_l1: last_l1_blk,
    };
    let update = ChainSyncStatusUpdate::new(status, fcm.cur_olstate.clone());
    status_channel.update_ol_chain_sync_status(update);

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

    let ol_blk_mgr = storage.ol_block();
    let mut slot = fcm.cur_best_block.slot() + 1;
    loop {
        let blkids = ol_blk_mgr.get_blocks_at_height_blocking(slot)?;
        if blkids.is_empty() {
            break;
        }

        warn!(%slot, ?blkids, "found extra L2 blocks");
        for blockid in blkids {
            let status = ol_blk_mgr.get_block_status_blocking(blockid)?;
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

#[expect(clippy::large_enum_variant, reason = "used for fork choice manager")]
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
                let last_l1_blk = L1BlockCommitment::new(
                    Height::from_consensus(fcm_state.cur_olstate.last_l1_height())
                        .expect("Invalid l1 height"),
                    *fcm_state.cur_olstate.last_l1_blkid(),
                );

                let status = ChainSyncStatus {
                    tip: fcm_state.cur_best_block,
                    prev_epoch: fcm_state.get_ol_state_prev_epoch(),
                    finalized_epoch: *fcm_state.chain_tracker.finalized_epoch(),
                    // FIXME this is a bit convoluted, could this be simpler?
                    safe_l1: last_l1_blk,
                };

                let update = ChainSyncStatusUpdate::new(status, fcm_state.cur_olstate.clone());
                trace!(%blkid, "publishing new ol_state");
                status_channel.update_ol_chain_sync_status(update);

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

fn handle_new_block(fcm_state: &mut ForkChoiceManager, bundle: &OLBlock) -> anyhow::Result<bool> {
    let slot = bundle.header().slot();
    let blkid = &bundle.header().compute_blkid();
    info!(%blkid, %slot, "handling new block");

    /*
      debug!(?fcm_state.cur_best_block);
      debug!(?fcm_state.chain_tracker);
      debug!(?fcm_state.cur_ol_state);
    */

    // First, decide if the block seems correctly signed and we haven't
    // already marked it as invalid.
    let check_res = check_ol_block_proposal_valid(blkid, bundle, fcm_state.params.rollup());
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
        fcm_state.storage.ol_block().as_ref(),
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
    debug!(?fcm_state.cur_olstate);

    res
}

/// Returns if we should switch to the new fork.  This is dependent on our
/// current tip and any of the competing forks.  It's "sticky" in that it'll try
/// to stay where we currently are unless there's a definitely-better fork.
fn pick_best_block<'t>(
    cur_tip: &'t OLBlockId,
    tips_iter: impl Iterator<Item = &'t OLBlockId>,
    ol_block_mgr: &OLBlockManager,
) -> Result<&'t OLBlockId, Error> {
    let mut best_tip = cur_tip;
    let mut best_block = ol_block_mgr
        .get_block_data_blocking(*best_tip)?
        .ok_or(Error::MissingL2Block(*best_tip))?;

    // The implementation of this will only switch to a new tip if it's a higher
    // height than our current tip.  We'll make this more sophisticated in the
    // future if we have a more sophisticated consensus protocol.
    for other_tip in tips_iter {
        if other_tip == cur_tip {
            continue;
        }

        let other_block = ol_block_mgr
            .get_block_data_blocking(*other_tip)?
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
    bundle: &OLBlock,
) -> anyhow::Result<()> {
    match update {
        // Easy case.
        TipUpdate::ExtendTip(_cur, _new) => {
            // TODO: what's the relation between _new and bundle
            // Update the tip block in the FCM state.
            let blk_cmmt =
                L2BlockCommitment::new(bundle.header().slot(), bundle.header().compute_blkid());
            let ol_state = fcm_state
                .storage
                .ol_state()
                .get_toplevel_ol_state_blocking(blk_cmmt)?
                .ok_or(DbError::MissingStateInstance)?;

            fcm_state.update_tip_block(blk_cmmt, ol_state)?;

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
                debug!(%pivot_blkid, %pivot_slot, "rolling back ol_state");
                revert_ol_state_to_block(&pivot_block, fcm_state)?;
            } else {
                warn!("got a reorg that didn't roll back to an earlier pivot");
            }

            // TODO any cleanup?

            Ok(())
        }

        TipUpdate::Revert(_cur, new) => {
            let slot = fcm_state.get_block_slot(&new)?;
            let block = L2BlockCommitment::new(slot, new);
            revert_ol_state_to_block(&block, fcm_state)?;
            Ok(())
        }
    }
}

/// Safely reverts the in-memory ol_state to a particular block, then rolls
/// back the writes on-disk.
fn revert_ol_state_to_block(
    block: &L2BlockCommitment,
    fcm_state: &mut ForkChoiceManager,
) -> anyhow::Result<()> {
    // Fetch the old state from the database and store in memory.  This
    // is also how  we validate that we actually *can* revert to this
    // block.
    let blkid = *block.blkid();
    let new_state = fcm_state
        .storage
        .ol_state()
        .get_toplevel_ol_state_blocking(*block)?
        .ok_or(Error::MissingBlockChainstate(blkid))?;
    let _ = fcm_state.update_tip_block(*block, new_state);

    // FIXME: Rollback the writes on the database that we no longer need.

    Ok(())
}

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
    let Some((_idx, next_finalizable_epoch)) = fcm_state.find_latest_pending_finalizable_epoch()
    else {
        // no new blocks to finalize
        return Ok(None);
    };

    fcm_state.finalize_epoch(&next_finalizable_epoch)?;

    info!(?next_finalizable_epoch, "updated finalized tip");
    //trace!(?fin_report, "finalization report");
    // TODO do something with the finalization report?

    Ok(Some(next_finalizable_epoch))
}

/// Checks OL block's credential to ensure that it was authentically proposed.
pub fn check_ol_block_proposal_valid(
    _blkid: &L2BlockId,
    block: &OLBlock,
    params: &RollupParams,
) -> anyhow::Result<()> {
    // If it's not the genesis block, check that the block is correctly signed.
    if block.header().slot() > 0 {
        let sig = block
            .signed_header()
            .signature()
            .expect("signature not present");
        let msg: Buf32 = block.header().compute_blkid().into();
        let is_valid = match params.cred_rule {
            CredRule::Unchecked => true,
            CredRule::SchnorrKey(pubkey) => verify_schnorr_sig(sig, &msg, &pubkey),
        };
        if !is_valid {
            return Err(anyhow!("block creds check failed"));
        }
    }

    Ok(())
}
