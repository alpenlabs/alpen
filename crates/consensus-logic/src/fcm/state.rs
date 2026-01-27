use std::{collections::VecDeque, sync::Arc, thread::sleep, time};

use anyhow::anyhow;
use strata_chain_worker_new::WorkerResult;
use strata_db_types::{types::CheckpointConfStatus, DbError};
use strata_ledger_types::IStateAccessor;
use strata_ol_state_types::OLState;
use strata_primitives::{EpochCommitment, L2BlockCommitment, OLBlockCommitment, OLBlockId};
use strata_service::ServiceState;
use strata_storage::OLBlockManager;
use tracing::{debug, warn};

use crate::{
    errors::Error, fcm::context::FcmContext, unfinalized_tracker::UnfinalizedBlockTracker,
};

#[expect(
    missing_debug_implementations,
    reason = "Debug is not applicable to FcmState"
)]
pub struct FcmState {
    ctx: FcmContext,
    inner_state: FcmInnerState,
}

impl FcmState {
    pub(crate) fn cur_ol_state(&self) -> Arc<OLState> {
        self.inner_state.cur_olstate.clone()
    }
    /// Gets the most recently finalized epoch, even if it's one that we haven't
    /// accepted as a new base yet due to missing intermediary blocks.
    fn get_most_recently_finalized_epoch(&self) -> &EpochCommitment {
        self.inner_state
            .epochs_pending_finalization
            .back()
            .unwrap_or(self.inner_state.chain_tracker.finalized_epoch())
    }

    /// Does handling to accept an epoch as finalized before we've actually validated it.
    pub(crate) fn attach_epoch_pending_finalization(&mut self, epoch: EpochCommitment) -> bool {
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

        self.inner_state
            .epochs_pending_finalization
            .push_back(epoch);

        true
    }

    pub(crate) fn chain_tracker(&self) -> &UnfinalizedBlockTracker {
        &self.inner_state.chain_tracker
    }

    pub(crate) fn cur_best_block(&self) -> OLBlockCommitment {
        self.inner_state.cur_best_block
    }

    pub(crate) fn chain_tracker_mut(&mut self) -> &mut UnfinalizedBlockTracker {
        &mut self.inner_state.chain_tracker
    }

    pub(crate) async fn update_tip_block(
        &mut self,
        block: OLBlockCommitment,
        state: Arc<OLState>,
    ) -> WorkerResult<()> {
        self.inner_state.cur_best_block = block;
        self.inner_state.cur_olstate = state;
        self.ctx().chain_worker().update_safe_tip(block).await
    }

    pub(crate) fn find_latest_pending_finalizable_epoch(&self) -> Option<(usize, EpochCommitment)> {
        // the latest epoch which we have processed and is safe to finalize
        // If prev epoch is null return None
        let prev_epoch = self.inner_state.cur_olstate.cur_epoch().saturating_sub(1);
        if prev_epoch == 0 {
            return None;
        }
        self.inner_state
            .epochs_pending_finalization
            .iter()
            .enumerate()
            .rev()
            .find(|(_, epoch)| epoch.epoch() <= prev_epoch)
            .map(|(a, b)| (a, *b))
    }

    pub(crate) async fn finalize_epoch(&mut self, epoch: EpochCommitment) -> anyhow::Result<()> {
        // Safety check.
        let csm_status = self.ctx().csm_monitor().get_current();
        let fin_epoch = csm_status
            .last_finalized_epoch
            .unwrap_or(EpochCommitment::null());
        if epoch.epoch() < fin_epoch.epoch() {
            return Err(Error::FinalizeOldEpoch(epoch, fin_epoch).into());
        }

        // Do the leg work of applying the finalization.
        self.ctx().chain_worker().finalize_epoch(epoch).await?;

        // Now update the in memory bookkeeping about it.
        self.chain_tracker_mut().update_finalized_epoch(&epoch)?;

        // Clear out old pending entries.
        self.clear_pending_epochs(epoch)?;

        Ok(())
    }

    fn clear_pending_epochs(&mut self, epoch: EpochCommitment) -> anyhow::Result<()> {
        let epoch_pending_fin = &mut self.inner_state.epochs_pending_finalization;
        while epoch_pending_fin
            .front()
            .is_some_and(|e| e.epoch() <= epoch.epoch())
        {
            epoch_pending_fin
                .pop_front()
                .ok_or(anyhow!("pop on empty epoch_pending dequeue"))?;
        }
        Ok(())
    }

    pub(crate) async fn get_block_slot(&self, blkid: OLBlockId) -> anyhow::Result<u64> {
        // FIXME this comes from old code that said "this is horrible but it makes our current use
        // case much faster, see below"
        if blkid == *self.cur_best_block().blkid() {
            return Ok(self.cur_best_block().slot());
        }

        // FIXME we should have some in-memory cache of blkid->height, although now that we use the
        // manager this is less significant because we're cloning what's already in memory
        let block = self
            .ctx()
            .storage()
            .ol_block()
            .get_block_data_async(blkid)
            .await?
            .ok_or(Error::MissingL2Block(blkid))?;
        Ok(block.header().slot())
    }
}

impl FcmState {
    pub(crate) fn new(ctx: FcmContext, inner_state: FcmInnerState) -> Self {
        Self { ctx, inner_state }
    }

    pub(crate) fn ctx(&self) -> &FcmContext {
        &self.ctx
    }
}

impl ServiceState for FcmState {
    // FIXME: these methods should really be within `Service` trait
    fn name(&self) -> &str {
        "fcm"
    }

    fn span_prefix(&self) -> &str {
        "fcm"
    }
}

#[derive(Debug)]
pub(crate) struct FcmInnerState {
    chain_tracker: UnfinalizedBlockTracker,
    cur_best_block: L2BlockCommitment,
    cur_olstate: Arc<OLState>,
    epochs_pending_finalization: VecDeque<EpochCommitment>,
}

impl FcmInnerState {
    pub(crate) fn new(
        chain_tracker: UnfinalizedBlockTracker,
        cur_best_block: L2BlockCommitment,
        cur_olstate: Arc<OLState>,
    ) -> Self {
        Self {
            chain_tracker,
            cur_best_block,
            cur_olstate,
            epochs_pending_finalization: VecDeque::new(),
        }
    }
}

/// Creates the forkchoice manager state from a database and rollup params.
pub async fn init_fcm_service_state(fcm_ctx: FcmContext) -> anyhow::Result<FcmState> {
    // Load data about the last finalized block so we can use that to initialize
    // the finalized tracker.

    let storage = fcm_ctx.storage().clone();
    let genesis_blkid = loop {
        if let Some(blkcommt) = storage.ol_block().get_canonical_block_at_async(0).await? {
            break *blkcommt.blkid();
        }
        sleep(time::Duration::from_secs(1));
    };

    let csm_finalized_epoch = storage
        .ol_state()
        .get_latest_toplevel_ol_state_async()
        .await?
        .map(|(_, ols)| *ols.previous_epoch())
        .unwrap_or(EpochCommitment::new(0, 0, genesis_blkid));
    // Special case for null previouse epoch. Because null epoch has last tip id Buf32::zero() which
    // won't be in the db
    let csm_finalized_epoch = if csm_finalized_epoch.is_null() {
        EpochCommitment::new(0, 0, genesis_blkid)
    } else {
        csm_finalized_epoch
    };

    // TODO: correctly calculate finalized epoch
    let finalized_epoch = csm_finalized_epoch;

    debug!(?finalized_epoch, "loading from finalized block...");

    // Populate the unfinalized block tracker.
    let mut chain_tracker = UnfinalizedBlockTracker::new_empty(finalized_epoch);
    chain_tracker.load_unfinalized_blocks(storage.ol_block().as_ref())?;

    let cur_tip_block = determine_start_tip(&chain_tracker, storage.ol_block()).await?;
    debug!(?chain_tracker, "init chain tracker");

    // Load in that block's ol_state.
    let tip_blkid = cur_tip_block;
    let ol_state = storage
        .ol_state()
        .get_toplevel_ol_state_async(tip_blkid)
        .await?
        .ok_or(DbError::MissingSlotWriteBatch(*tip_blkid.blkid()))?;

    let fcm_inner = FcmInnerState::new(chain_tracker, cur_tip_block, ol_state);

    // Actually assemble the forkchoice manager state.
    let mut fcm = FcmState::new(fcm_ctx, fcm_inner);

    if finalized_epoch != csm_finalized_epoch {
        // csm is ahead of ol_state
        // search for all pending checkpoints
        for epoch in finalized_epoch.epoch()..=csm_finalized_epoch.epoch() {
            let Some(checkpoint_entry) =
                // TODO: use new checkpoint type and db(to be done in another ticket)
                storage.checkpoint().get_checkpoint(epoch as u64).await?
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
async fn determine_start_tip(
    unfin: &UnfinalizedBlockTracker,
    ol_block_mgr: &OLBlockManager,
) -> anyhow::Result<L2BlockCommitment> {
    let mut iter = unfin.chain_tips_iter();

    let mut best = iter.next().expect("fcm: no chain tips");
    let mut best_slot = ol_block_mgr
        .get_block_data_async(*best)
        .await?
        .ok_or(Error::MissingL2Block(*best))?
        .header()
        .slot();

    // Iterate through the remaining elements and choose.
    for blkid in iter {
        let blkid_slot = ol_block_mgr
            .get_block_data_async(*blkid)
            .await?
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
