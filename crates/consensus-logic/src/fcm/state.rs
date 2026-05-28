use std::{collections::VecDeque, sync::Arc, time};

use anyhow::anyhow;
use metrics::{counter, gauge};
use strata_ol_state_types::OLState;
use strata_predicate::PredicateKey;
use strata_primitives::{EpochCommitment, L2BlockCommitment, OLBlockCommitment, OLBlockId};
use strata_service::ServiceState;
use tokio::time::sleep;
use tracing::{debug, warn};

use crate::{
    errors::Error,
    fcm::context::{FcmContext, FcmStorage},
    unfinalized_tracker::UnfinalizedBlockTracker,
};

/// Runtime container for the FCM service.
///
/// `sequencer_predicate` is immutable launch configuration. The mutable
/// fork-choice state lives in [`FcmInnerState`].
pub(crate) struct FcmServiceState<C: FcmContext> {
    ctx: Arc<C>,
    sequencer_predicate: PredicateKey,
    inner_state: FcmInnerState,
}

impl<C: FcmContext> FcmServiceState<C> {
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
        self.record_pending_epochs();

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
    ) -> anyhow::Result<()> {
        self.inner_state.cur_best_block = block;
        self.inner_state.cur_olstate = state;
        gauge!("strata_ol_tip_slot").set(block.slot() as f64);
        self.ctx().update_safe_tip(block).await
    }

    pub(crate) fn find_latest_pending_finalizable_epoch(&self) -> Option<(usize, EpochCommitment)> {
        // the latest epoch which we have processed and is safe to finalize
        // If prev epoch is null return None
        let prev_epoch = self
            .inner_state
            .cur_olstate
            .epoch_state()
            .cur_epoch()
            .saturating_sub(1);
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
        let fin_epoch = self
            .ctx()
            .last_finalized_epoch()
            .unwrap_or(EpochCommitment::null());
        if epoch.epoch() < fin_epoch.epoch() {
            return Err(Error::FinalizeOldEpoch(epoch, fin_epoch).into());
        }

        // Do the leg work of applying the finalization.
        self.ctx().finalize_epoch(epoch).await?;

        // Now update the in memory bookkeeping about it.
        self.chain_tracker_mut().update_finalized_epoch(&epoch)?;

        // Clear out old pending entries.
        self.clear_pending_epochs(epoch)?;

        counter!("strata_fcm_epochs_finalized_total").increment(1);
        gauge!("strata_fcm_finalized_epoch").set(epoch.epoch() as f64);
        gauge!("strata_fcm_finalized_slot").set(epoch.last_slot() as f64);

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
        self.record_pending_epochs();
        Ok(())
    }

    fn record_pending_epochs(&self) {
        gauge!("strata_fcm_pending_epochs")
            .set(self.inner_state.epochs_pending_finalization.len() as f64);
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
            .get_ol_block(blkid)
            .await?
            .ok_or(Error::MissingOLBlock(blkid))?;
        Ok(block.header().slot())
    }
}

impl<C: FcmContext> FcmServiceState<C> {
    pub(crate) fn new(
        ctx: Arc<C>,
        sequencer_predicate: PredicateKey,
        inner_state: FcmInnerState,
    ) -> Self {
        Self {
            ctx,
            sequencer_predicate,
            inner_state,
        }
    }

    pub(crate) fn ctx(&self) -> &C {
        self.ctx.as_ref()
    }

    pub(crate) fn sequencer_predicate(&self) -> &PredicateKey {
        &self.sequencer_predicate
    }
}

impl<C: FcmContext> ServiceState for FcmServiceState<C> {
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

/// Creates the forkchoice manager state from the FCM context and sequencer
/// predicate.
pub(crate) async fn init_fcm_service_state<C: FcmContext>(
    sequencer_predicate: PredicateKey,
    fcm_ctx: Arc<C>,
) -> anyhow::Result<FcmServiceState<C>> {
    // Load data about the last finalized block so we can use that to initialize
    // the finalized tracker.

    let genesis_blkid = loop {
        if let Some(blkcommt) = fcm_ctx.get_canonical_block_at(0).await? {
            break *blkcommt.blkid();
        }
        let _ = sleep(time::Duration::from_secs(1)).await;
    };

    let finalized_epoch = fcm_ctx
        .last_finalized_epoch()
        .unwrap_or(EpochCommitment::new(0, 0, genesis_blkid));

    debug!(?finalized_epoch, "loading from finalized block...");

    // Populate the unfinalized block tracker.
    let mut chain_tracker = UnfinalizedBlockTracker::new_empty(finalized_epoch);
    chain_tracker
        .load_unfinalized_ol_blocks_async(fcm_ctx.as_ref())
        .await?;

    let cur_tip_block = determine_start_tip(&chain_tracker, fcm_ctx.as_ref()).await?;
    debug!(?chain_tracker, "init chain tracker");

    // Load in that block's ol_state.
    let tip_blkid = cur_tip_block;
    let ol_state = fcm_ctx
        .get_toplevel_ol_state(tip_blkid)
        .await?
        .ok_or(Error::MissingOLState(tip_blkid))?;

    let fcm_inner = FcmInnerState::new(chain_tracker, cur_tip_block, ol_state);
    gauge!("strata_ol_tip_slot").set(cur_tip_block.slot() as f64);
    gauge!("strata_fcm_finalized_epoch").set(finalized_epoch.epoch() as f64);
    gauge!("strata_fcm_finalized_slot").set(finalized_epoch.last_slot() as f64);
    gauge!("strata_fcm_pending_epochs").set(0.0);

    // Actually assemble the forkchoice manager state.
    Ok(FcmServiceState::new(
        fcm_ctx,
        sequencer_predicate,
        fcm_inner,
    ))
}

/// Determines the starting chain tip.  For now, this is just the block with the
/// highest index, choosing the lowest ordered blockid in the case of ties.
async fn determine_start_tip(
    unfin: &UnfinalizedBlockTracker,
    storage: &(impl FcmStorage + ?Sized),
) -> anyhow::Result<L2BlockCommitment> {
    let mut iter = unfin.chain_tips_iter();

    let mut best = iter.next().expect("fcm: no chain tips");
    let mut best_slot = storage
        .get_ol_block(*best)
        .await?
        .ok_or(Error::MissingOLBlock(*best))?
        .header()
        .slot();

    // Iterate through the remaining elements and choose.
    for blkid in iter {
        let blkid_slot = storage
            .get_ol_block(*blkid)
            .await?
            .ok_or(Error::MissingOLBlock(*blkid))?
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
