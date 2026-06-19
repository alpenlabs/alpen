use std::{collections::VecDeque, iter, mem, sync::Arc, time};

use metrics::{counter, gauge};
use strata_identifiers::Slot;
use strata_ol_state_types::OLState;
use strata_predicate::PredicateKey;
use strata_primitives::{EpochCommitment, OLBlockCommitment, OLBlockId};
use strata_service::ServiceState;
use tokio::time::sleep;
use tracing::{debug, warn};

use crate::{
    errors::Error,
    fcm::context::{FcmContext, FcmStorage},
    unfinalized_tracker::UnfinalizedBlockTracker,
};

type CanonicalSuffix = Vec<OLBlockId>;

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

    /// Gets the latest finalized epoch observed by FCM.
    ///
    /// This may be newer than [`UnfinalizedBlockTracker::finalized_epoch`] when
    /// CSM has reported a finalized epoch but FCM's finalized epoch has not
    /// advanced to it yet because the terminal block is not finalizable.
    pub(crate) fn latest_observed_finalized_epoch(&self) -> &EpochCommitment {
        self.inner_state
            .epochs_pending_finalization
            .back()
            .unwrap_or(self.inner_state.chain_tracker.finalized_epoch())
    }

    /// Records a finalized epoch observed from FCM's context.
    ///
    /// Returns whether the epoch was queued for FCM finalization.
    pub(crate) fn record_observed_finalized_epoch(&mut self, epoch: EpochCommitment) -> bool {
        let latest_observed_finalized_epoch = self.latest_observed_finalized_epoch();

        if epoch.is_null() {
            warn!("tried to finalize null epoch");
            return false;
        }

        // Some checks to make sure we don't go backwards.
        if latest_observed_finalized_epoch.last_slot() > 0 {
            let epoch_advances = epoch.epoch() > latest_observed_finalized_epoch.epoch();
            let block_advances = epoch.last_slot() > latest_observed_finalized_epoch.last_slot();

            if epoch == *latest_observed_finalized_epoch {
                debug!(
                    ?latest_observed_finalized_epoch,
                    "finalized epoch already observed, ignoring"
                );
                return false;
            }

            if !epoch_advances || !block_advances {
                let epoch_regresses = epoch.epoch() < latest_observed_finalized_epoch.epoch();
                let block_regresses =
                    epoch.last_slot() < latest_observed_finalized_epoch.last_slot();
                if epoch_regresses && block_regresses {
                    warn!(
                        ?latest_observed_finalized_epoch,
                        received = ?epoch,
                        "received out-of-order epoch"
                    );
                } else {
                    warn!(
                        ?latest_observed_finalized_epoch,
                        received = ?epoch,
                        "received inconsistent epoch ordering"
                    );
                }
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

    pub(crate) fn take_startup_replay_candidates(&mut self) -> Vec<OLBlockId> {
        mem::take(&mut self.inner_state.startup_replay_candidates)
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
        self.clear_pending_epochs(epoch);

        counter!("strata_fcm_epochs_finalized_total").increment(1);
        gauge!("strata_fcm_finalized_epoch").set(epoch.epoch() as f64);
        gauge!("strata_fcm_finalized_slot").set(epoch.last_slot() as f64);

        Ok(())
    }

    fn clear_pending_epochs(&mut self, epoch: EpochCommitment) {
        let epoch_pending_fin = &mut self.inner_state.epochs_pending_finalization;
        while epoch_pending_fin
            .front()
            .is_some_and(|e| e.epoch() <= epoch.epoch())
        {
            epoch_pending_fin
                .pop_front()
                .expect("front checked before popping pending finalized epoch");
        }
        self.record_pending_epochs();
    }

    fn record_pending_epochs(&self) {
        gauge!("strata_fcm_pending_epochs")
            .set(self.inner_state.epochs_pending_finalization.len() as f64);
    }

    pub(crate) async fn get_block_slot(&self, blkid: OLBlockId) -> anyhow::Result<u64> {
        // FIXME(STR-3673): this comes from old code that said "this is horrible but it makes our
        // current use case much faster, see below"
        if blkid == *self.cur_best_block().blkid() {
            return Ok(self.cur_best_block().slot());
        }

        // FIXME(STR-3673): we should have some in-memory cache of blkid->height, although now that
        // we use the manager this is less significant because we're cloning what's already
        // in memory
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
    // FIXME(STR-3673): these methods should really be within `Service` trait
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
    cur_best_block: OLBlockCommitment,
    cur_olstate: Arc<OLState>,
    startup_replay_candidates: Vec<OLBlockId>,
    epochs_pending_finalization: VecDeque<EpochCommitment>,
}

impl FcmInnerState {
    pub(crate) fn new(
        chain_tracker: UnfinalizedBlockTracker,
        cur_best_block: OLBlockCommitment,
        cur_olstate: Arc<OLState>,
        startup_replay_candidates: Vec<OLBlockId>,
    ) -> Self {
        Self {
            chain_tracker,
            cur_best_block,
            cur_olstate,
            startup_replay_candidates,
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
    let startup_replay_candidates = chain_tracker
        .load_unfinalized_ol_blocks_async(fcm_ctx.as_ref())
        .await?;

    let cur_tip_block = determine_start_tip(&chain_tracker)?;
    debug!(?chain_tracker, "init chain tracker");

    // Update the canonical blocks index just in case this might have drifted during the restarts.
    reconcile_canonical_blocks_index(&chain_tracker, cur_tip_block, fcm_ctx.as_ref()).await?;

    // Load in that block's ol_state.
    let tip_blkid = cur_tip_block;
    let ol_state = fcm_ctx
        .get_toplevel_ol_state(tip_blkid)
        .await?
        .ok_or(Error::MissingOLState(tip_blkid))?;

    let fcm_inner = FcmInnerState::new(
        chain_tracker,
        cur_tip_block,
        ol_state,
        startup_replay_candidates,
    );
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

/// Determines the starting chain tip by choosing the highest-slot tip, using
/// the lowest ordered block ID as the tie-breaker.
fn determine_start_tip(unfin: &UnfinalizedBlockTracker) -> Result<OLBlockCommitment, Error> {
    // Unfinalized block tracker only loads blocks which are valid and exist in db so no need to
    // check for db existence at this point.
    unfin
        .chain_tip_blocks_iter()
        .max_by(|a, b| {
            a.slot()
                .cmp(&b.slot())
                .then_with(|| b.blkid().cmp(a.blkid()))
        })
        .ok_or(Error::FcmNoChainTips)
}

/// Rewrites the canonical index to match the reconstructed chain.
///
/// Only the unfinalized suffix is rewritten; slots at or below the finalized tip are left untouched
/// since finalized history never forks.
pub(crate) async fn reconcile_canonical_blocks_index(
    unfin: &UnfinalizedBlockTracker,
    tip: OLBlockCommitment,
    storage: &(impl FcmStorage + ?Sized),
) -> Result<(), Error> {
    let (pivot_slot, canon_blocks) = canonical_blocks_from_tip(unfin, tip, storage).await?;
    let Some(start_slot) = pivot_slot.checked_add(1) else {
        // An empty suffix at the max slot means nothing above the tip to truncate.
        if canon_blocks.is_empty() {
            return Ok(());
        }
        return Err(Error::FcmCanonicalSuffixAboveMaxSlot(pivot_slot));
    };
    storage
        .replace_canonical_suffix_from(start_slot, canon_blocks)
        .await?;
    Ok(())
}

/// Walks from `tip` toward the finalized tip until it reaches the first block
/// already recorded as canonical.
async fn canonical_blocks_from_tip(
    unfin: &UnfinalizedBlockTracker,
    tip: OLBlockCommitment,
    storage: &(impl FcmStorage + ?Sized),
) -> Result<(Slot, CanonicalSuffix), Error> {
    let parent_chain = construct_validated_parent_chain(unfin, tip)?;
    let finalized_tip = *unfin.finalized_tip();

    if parent_chain.last() != Some(&finalized_tip) {
        return Err(Error::FcmCanonicalChainNotFinalized {
            tip: *tip.blkid(),
            finalized_tip,
        });
    }

    let mut suffix = Vec::new();

    for blkid in parent_chain {
        let slot = tracker_slot(unfin, blkid)?;

        let canonical = storage.get_canonical_block_at(slot).await?;
        if canonical.map(|c| *c.blkid()) == Some(blkid) {
            suffix.reverse();
            return Ok((slot, suffix));
        }

        if blkid == finalized_tip {
            let finalized_slot = unfin.finalized_epoch().last_slot();
            return Err(Error::FcmCanonicalFinalizedMismatch {
                finalized_slot,
                canonical,
                finalized_tip,
            });
        }

        suffix.push(blkid);
    }

    unreachable!("validated parent chain always includes the finalized tip")
}

fn construct_validated_parent_chain(
    unfin: &UnfinalizedBlockTracker,
    tip: OLBlockCommitment,
) -> Result<Vec<OLBlockId>, Error> {
    let mut chain = Vec::new();
    let mut child_slot: Option<Slot> = None;
    let parent_chain =
        iter::successors(Some(*tip.blkid()), |blkid| unfin.get_parent(blkid).copied());

    for blkid in parent_chain {
        let slot = tracker_slot(unfin, blkid)?;
        ensure_parent_slot_descends(slot, child_slot, tip)?;
        child_slot = Some(slot);
        chain.push(blkid);
    }

    Ok(chain)
}

fn tracker_slot(unfin: &UnfinalizedBlockTracker, blkid: OLBlockId) -> Result<Slot, Error> {
    unfin
        .get_slot(&blkid)
        .ok_or(Error::FcmCanonicalTrackerMissingBlock(blkid))
}

fn ensure_parent_slot_descends(
    parent_slot: Slot,
    child_slot: Option<Slot>,
    tip: OLBlockCommitment,
) -> Result<(), Error> {
    if let Some(child_slot) = child_slot {
        if parent_slot >= child_slot {
            return Err(Error::FcmCanonicalParentSlotNotDescending {
                parent_slot,
                child_slot,
                tip,
            });
        }
    }

    Ok(())
}
