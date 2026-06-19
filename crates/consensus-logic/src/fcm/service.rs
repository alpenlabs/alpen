use std::{marker::PhantomData, sync::Arc};

use anyhow::{anyhow, Context};
use metrics::{counter, histogram};
use serde::Serialize;
use strata_csm_types::CheckpointState;
use strata_db_types::traits::BlockStatus;
use strata_identifiers::Slot;
use strata_ol_chain_types_new::{
    sequencer_predicate_requires_signature, verify_sequencer_predicate_signature, OLBlock,
};
use strata_predicate::PredicateKey;
use strata_primitives::{Buf32, EpochCommitment, L1BlockCommitment, OLBlockCommitment, OLBlockId};
use strata_service::{AsyncService, Response, Service, ServiceBuilder, ServiceMonitor};
use strata_status::OLSyncStatus;
use strata_tasks::TaskExecutor;
use tokio::sync::{
    mpsc::{channel as mpsc_channel, Sender},
    watch,
};
use tracing::{debug, error, info, trace, warn};

use super::state::init_fcm_service_state;
use crate::{
    errors::Error,
    fcm::{
        context::{FcmContext, FcmStorage},
        input::FcmEvent,
        state::FcmServiceState,
    },
    message::ForkChoiceMessage,
    tip_update::{compute_tip_update, TipUpdate},
    FcmInput,
};

#[derive(Clone, Debug)]
pub struct FcmServiceHandle {
    fcm_tx: Sender<ForkChoiceMessage>,
    service_monitor: ServiceMonitor<FcmStatus>,
}

impl FcmServiceHandle {
    pub fn submit_chain_tip_msg_blocking(&self, msg: ForkChoiceMessage) -> bool {
        self.fcm_tx.blocking_send(msg).is_ok()
    }

    pub async fn submit_chain_tip_msg_async(&self, msg: ForkChoiceMessage) -> bool {
        self.fcm_tx.send(msg).await.is_ok()
    }

    pub fn fcm_status(&self) -> FcmStatus {
        self.service_monitor.get_current()
    }
}

pub async fn start_fcm_service<C: FcmContext>(
    sequencer_predicate: PredicateKey,
    fcm_ctx: Arc<C>,
    checkpoint_state_rx: watch::Receiver<CheckpointState>,
    texec: Arc<TaskExecutor>,
) -> anyhow::Result<FcmServiceHandle> {
    // initialize fcm state
    let fcm_state = init_fcm_service_state(sequencer_predicate, fcm_ctx).await?;

    let (fcm_tx, fcm_rx) = mpsc_channel::<ForkChoiceMessage>(64);
    let fcm_input = FcmInput::new(fcm_rx, checkpoint_state_rx);

    let service_monitor = ServiceBuilder::<FcmService<C>, FcmInput>::new()
        .with_state(fcm_state)
        .with_input(fcm_input)
        .launch_async("fcm", texec.as_ref())
        .await?;

    Ok(FcmServiceHandle {
        service_monitor,
        fcm_tx,
    })
}

#[derive(Clone, Debug)]
pub(crate) struct FcmService<C: FcmContext>(PhantomData<C>);

impl<C: FcmContext> Default for FcmService<C> {
    fn default() -> Self {
        Self(PhantomData)
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct FcmStatus;

impl<C: FcmContext> Service for FcmService<C> {
    type Msg = FcmEvent;
    type State = FcmServiceState<C>;
    type Status = FcmStatus;

    fn get_status(_s: &Self::State) -> Self::Status {
        FcmStatus
    }
}

impl<C: FcmContext> AsyncService for FcmService<C> {
    async fn on_launch(state: &mut Self::State) -> anyhow::Result<()> {
        let startup_replay_candidates = state.take_startup_replay_candidates();
        if startup_replay_candidates.is_empty() {
            return Ok(());
        }

        let replay_candidate_count = startup_replay_candidates.len();
        for blkid in startup_replay_candidates {
            let msg = ForkChoiceMessage::NewBlock(blkid);
            process_fc_message(&msg, state)
                .await
                .with_context(|| format!("failed to replay startup OL block {blkid}"))?;
        }

        debug!(
            replay_candidate_count,
            "processed startup replay candidates"
        );
        Ok(())
    }

    async fn before_shutdown(
        _state: &mut Self::State,
        _err: Option<&anyhow::Error>,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    async fn process_input(
        fcm_state: &mut Self::State,
        input: Self::Msg,
    ) -> anyhow::Result<Response> {
        match &input {
            FcmEvent::NewFcmMsg(m) => process_fc_message(m, fcm_state).await?,
            FcmEvent::NewStateUpdate => handle_new_state_update(fcm_state).await?,
            FcmEvent::Abort => return Ok(Response::ShouldExit),
        };
        Ok(Response::Continue)
    }
}

async fn process_fc_message<C: FcmContext>(
    msg: &ForkChoiceMessage,
    fcm_state: &mut FcmServiceState<C>,
) -> anyhow::Result<()> {
    match msg {
        ForkChoiceMessage::NewBlock(blkid) => {
            strata_common::check_bail_trigger(strata_common::BAIL_FCM_NEW_BLOCK);

            let block_bundle = fcm_state
                .ctx()
                .get_ol_block(*blkid)
                .await?
                .ok_or(Error::MissingOLBlock(*blkid))?;

            let slot = block_bundle.header().slot();
            info!(%slot, %blkid, "processing new block");

            let ok = match handle_new_block(fcm_state, &block_bundle).await {
                Ok(v) => v,
                Err(e) => {
                    // Really we shouldn't emit this error unless there's a
                    // problem checking the block in general and it could be
                    // valid or invalid, but we're kinda sloppy with errors
                    // here so let's try to avoid crashing the FCM task?
                    error!(
                        %slot,
                        %blkid,
                        err = ?e,
                        "error processing block, interpreting as invalid"
                    );
                    false
                }
            };

            let status = if ok {
                // check if any pending blocks can be finalized
                if let Err(err) = handle_epoch_finalization(fcm_state).await {
                    error!(%err, "failed to finalize epoch");
                }

                // Update status.
                let last_l1_blk = L1BlockCommitment::new(
                    fcm_state.cur_ol_state().epoch_state().last_l1_height(),
                    *fcm_state.cur_ol_state().epoch_state().last_l1_blkid(),
                );

                let cur_state = fcm_state.cur_ol_state();
                // Get prev epoch summary
                let prev_epoch_num = cur_state.epoch_state().cur_epoch().saturating_sub(1);
                let prev_epoch = fcm_state
                    .ctx()
                    .get_canonical_epoch_commitment_at(prev_epoch_num)
                    .await?
                    .ok_or(anyhow!(
                        "expected epoch commitment for previous epoch {} not in db",
                        prev_epoch_num
                    ))?;
                let finalized_epoch = *fcm_state.chain_tracker().finalized_epoch();
                let confirmed_epoch = fcm_state
                    .ctx()
                    .last_confirmed_epoch()
                    .unwrap_or(finalized_epoch);

                let canonical_tip = fcm_state.cur_best_block();
                let tip_block_data = fcm_state
                    .ctx()
                    .get_ol_block(*canonical_tip.blkid())
                    .await?
                    .ok_or(Error::MissingOLBlock(*canonical_tip.blkid()))?;
                let status = OLSyncStatus {
                    tip: canonical_tip,
                    tip_epoch: tip_block_data.header().epoch(),
                    tip_is_terminal: tip_block_data.header().is_terminal(),
                    prev_epoch,
                    confirmed_epoch,
                    finalized_epoch,
                    // FIXME(STR-3673): this is a bit convoluted, could this be simpler?
                    safe_l1: last_l1_blk,
                };

                trace!(%blkid, "publishing new ol_state");
                fcm_state.ctx().publish_sync_status(status);

                BlockStatus::Valid
            } else {
                // Emit invalid block warning.
                warn!(%blkid, "rejecting invalid block");
                BlockStatus::Invalid
            };

            let block = OLBlockCommitment::new(slot, *blkid);
            set_block_status_and_clear_invalid_high_watermark(
                fcm_state,
                &block_bundle,
                block,
                status,
            )
            .await?;
            if ok {
                counter!("strata_fcm_blocks_accepted_total").increment(1);
            } else {
                counter!("strata_fcm_blocks_rejected_total").increment(1);
            }
        }
    }

    Ok(())
}

async fn set_block_status_and_clear_invalid_high_watermark<C: FcmContext>(
    fcm_state: &FcmServiceState<C>,
    bundle: &OLBlock,
    block: OLBlockCommitment,
    status: BlockStatus,
) -> anyhow::Result<bool> {
    let updated = fcm_state
        .ctx()
        .set_block_status(*block.blkid(), status)
        .await?;

    if matches!(status, BlockStatus::Invalid) {
        // TODO(STR-2141): `BlockStatus::Invalid` also represents local execution failures.
        // Revisit high-watermark clearing once FCM distinguishes consensus-invalid blocks
        // from transient execution failures.

        // A rejected terminal block may have stored its epoch summary before
        // failing (the summary is the last exec step, but post-exec failures
        // can still invalidate the block afterwards). Drop it so it cannot
        // shadow the replacement terminal's summary in canonical lookups.
        // Keyed exactly by the rejected block's own commitment, this can
        // never touch another block's summary, so it runs regardless of the
        // high-watermark gate below.
        if bundle.header().is_terminal() {
            let summary_commitment =
                EpochCommitment::new(bundle.header().epoch(), block.slot(), *block.blkid());
            let deleted = fcm_state
                .ctx()
                .del_epoch_summary(summary_commitment)
                .await
                .inspect_err(|err| {
                    error!(
                        %block,
                        %err,
                        "failed to delete epoch summary of invalid OL terminal block"
                    );
                })
                .context("failed to delete epoch summary of invalid OL terminal block")?;
            if deleted {
                info!(%block, "deleted epoch summary of invalid OL terminal block");
            }
        }

        // The cleanup below only applies to the block the sequencer is
        // currently stuck on. An invalid block that is not the high-watermark
        // (e.g. a stale or forked proposal arriving after a valid block at
        // this slot was accepted) must not trigger a rollback: the indexing
        // rows past its parent slot belong to the accepted canonical chain.
        let high_watermark = fcm_state.ctx().get_block_high_watermark().await?;
        if high_watermark != Some(block) {
            debug!(%block, "invalid OL block is not the high-watermark; skipping indexing rollback");
            return Ok(updated);
        }

        // Drop any state-indexing writes the rejected block persisted before
        // it failed, so a replacement block at this slot doesn't conflict
        // against the indexing high-watermark. The high-watermark is advanced
        // when a block is stored and execution follows storage, so with the
        // rejected block at the high-watermark the epoch's indexing writes
        // past its parent slot can only be its own. Must land before the
        // high-watermark clear below, since the clear is what unblocks
        // building the replacement.
        let cutoff = OLBlockCommitment::new(
            block.slot().saturating_sub(1),
            *bundle.header().parent_blkid(),
        );
        fcm_state
            .ctx()
            .rollback_block_state_indexing(bundle.header().epoch(), cutoff)
            .await
            .inspect_err(|err| {
                error!(
                    %block,
                    %err,
                    "failed to roll back state indexing for invalid OL block; replacement generation for this slot remains blocked"
                );
            })
            .context("failed to roll back state indexing for invalid OL block")?;

        let cleared = fcm_state
            .ctx()
            .clear_block_high_watermark(block)
            .await
            .inspect_err(|err| {
                error!(
                    %block,
                    %err,
                    "failed to clear high-watermark for invalid OL block; replacement generation for this slot remains blocked"
                );
            })
            .context("failed to clear invalid OL block high-watermark")?;
        if cleared {
            info!(%block, "cleared invalid OL block high-watermark");
        }
    }

    Ok(updated)
}

async fn handle_new_state_update<C: FcmContext>(
    fcm_state: &mut FcmServiceState<C>,
) -> anyhow::Result<()> {
    let Some(observed_finalized_epoch) = fcm_state.ctx().last_finalized_epoch() else {
        debug!("got new CSM state, but finalized epoch still unset, ignoring");
        return Ok(());
    };

    let current_finalized_epoch = *fcm_state.chain_tracker().finalized_epoch();
    if observed_finalized_epoch == current_finalized_epoch {
        debug!(
            ?current_finalized_epoch,
            ?observed_finalized_epoch,
            "no new finalized epoch in CSM update"
        );
        return Ok(());
    }

    let latest_observed_finalized_epoch = *fcm_state.latest_observed_finalized_epoch();
    if observed_finalized_epoch == latest_observed_finalized_epoch {
        debug!(
            ?observed_finalized_epoch,
            "observed finalized epoch is already recorded, checking finalization progress"
        );
        check_finalization_progress(fcm_state, observed_finalized_epoch).await?;
        return Ok(());
    }

    if !fcm_state.record_observed_finalized_epoch(observed_finalized_epoch) {
        return Ok(());
    }

    info!(?observed_finalized_epoch, "observed new finalized epoch");
    check_finalization_progress(fcm_state, observed_finalized_epoch).await?;

    Ok(())
}

async fn check_finalization_progress<C: FcmContext>(
    fcm_state: &mut FcmServiceState<C>,
    observed_finalized_epoch: EpochCommitment,
) -> anyhow::Result<()> {
    match handle_epoch_finalization(fcm_state).await {
        Err(err) => {
            error!(%err, "failed to finalize epoch");
        }
        Ok(Some(finalized_epoch)) if finalized_epoch == observed_finalized_epoch => {
            debug!(
                ?finalized_epoch,
                "FCM caught up to observed finalized epoch"
            );
        }
        Ok(Some(finalized_epoch)) => {
            debug!(
                ?finalized_epoch,
                ?observed_finalized_epoch,
                "FCM finalized earlier recorded epoch; still behind observed finalized epoch"
            );
        }
        Ok(None) => {
            // there were no epochs that could be finalized
            debug!(?observed_finalized_epoch, "no finalization progress");
        }
    };

    Ok(())
}

async fn handle_new_block<C: FcmContext>(
    fcm_state: &mut FcmServiceState<C>,
    bundle: &OLBlock,
) -> anyhow::Result<bool> {
    let slot = bundle.header().slot();
    let blkid = &bundle.header().compute_blkid();
    info!(%blkid, %slot, "handling new block");

    // First, decide if the block seems correctly signed and we haven't
    // already marked it as invalid.
    if let Err(err) = check_ol_block_proposal_valid(blkid, bundle, fcm_state.sequencer_predicate())
    {
        warn!(%err, "rejecting block");
        return Ok(false);
    }

    // This stores the block output in the database, which lets us make queries
    // about it, at least until it gets reorged out by another block being
    // finalized.
    let bc = OLBlockCommitment::new(bundle.header().slot(), *blkid);
    let exec_ok = match fcm_state.ctx().try_exec_block(bc).await {
        Ok(()) => true,
        Err(err) => {
            // TODO(STR-2141): Need some way to distinguish an invalid block from a exec failure
            error!(%err, "try_exec_block failed");
            false
        }
    };

    if exec_ok {
        fcm_state
            .ctx()
            .set_block_status(*blkid, BlockStatus::Valid)
            .await?;
    } else {
        set_block_status_and_clear_invalid_high_watermark(
            fcm_state,
            bundle,
            bc,
            BlockStatus::Invalid,
        )
        .await?;
        return Ok(false);
    }

    // Insert block into pending block tracker and figure out if we
    // should switch to it as a potential head.  This returns if we
    // created a new tip instead of advancing an existing tip.
    let cur_tip = *fcm_state.cur_best_block().blkid();
    let new_tip = fcm_state.chain_tracker_mut().attach_block(
        bundle.header().slot(),
        *blkid,
        *bundle.header().parent_blkid(),
    )?;

    if new_tip {
        debug!(?blkid, "created new branching tip");
    }

    // Now decide what the new tip should be and figure out how to get there.
    let tips: Vec<OLBlockId> = fcm_state
        .chain_tracker()
        .chain_tips_iter()
        .copied()
        .collect();
    let best_block = pick_best_block_async(&cur_tip, &tips, fcm_state.ctx()).await?;

    // TODO(STR-3050): make configurable
    let depth = 100;

    let tip_update = compute_tip_update(&cur_tip, &best_block, depth, fcm_state.chain_tracker())?;
    let Some(tip_update) = tip_update else {
        // In this case there's no change.
        return Ok(true);
    };

    let tip_blkid = *tip_update.new_tip();
    debug!(%tip_blkid, "have new tip, applying update");

    // Apply the reorg.
    let res = match apply_tip_update(tip_update, fcm_state, bundle).await {
        Ok(()) => {
            info!(%tip_blkid, "new chain tip");

            Ok(true)
        }

        Err(e) => {
            warn!(err = ?e, "failed to compute CL STF");

            // TODO(STR-2170): the legacy chain worker surfaced a typed
            // `InvalidStateTsn` error that let us reject a bad block and remember
            // not to retry it (returning `Ok(false)`). The new OL STF path does
            // not yet expose such a detectable error, so for now we propagate all
            // apply failures. Restore block rejection once the OL chain worker
            // exposes an invalid-transition error to match on here.
            Err(e)
        }
    };

    res
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
async fn handle_epoch_finalization<C: FcmContext>(
    fcm_state: &mut FcmServiceState<C>,
) -> anyhow::Result<Option<EpochCommitment>> {
    let Some((_idx, next_finalizable_epoch)) = fcm_state.find_latest_pending_finalizable_epoch()
    else {
        // no new blocks to finalize
        return Ok(None);
    };

    fcm_state.finalize_epoch(next_finalizable_epoch).await?;

    info!(?next_finalizable_epoch, "advanced finalized epoch");

    Ok(Some(next_finalizable_epoch))
}

/// Checks OL block's credential to ensure that it was authentically proposed.
///
/// Slot-0 (genesis) blocks are not expected as proposals — genesis is fixed at node init.
pub fn check_ol_block_proposal_valid(
    blkid: &OLBlockId,
    block: &OLBlock,
    sequencer_predicate: &PredicateKey,
) -> Result<(), Error> {
    if block.header().slot() == 0 {
        return Err(Error::UnexpectedGenesisBlock(*blkid));
    }
    let sig = match block.signed_header().signature() {
        Some(sig) => sig,
        None if !sequencer_predicate_requires_signature(sequencer_predicate) => return Ok(()),
        None => return Err(Error::MissingBlockSignature(*blkid)),
    };
    let msg: Buf32 = block.header().compute_blkid().into();
    let is_valid = verify_sequencer_predicate_signature(sequencer_predicate, &msg, sig);
    if !is_valid {
        return Err(Error::InvalidBlockSignature(*blkid));
    }
    Ok(())
}

async fn pick_best_block_async<S>(
    cur_tip: &OLBlockId,
    tips: &[OLBlockId],
    storage: &S,
) -> Result<OLBlockId, Error>
where
    S: FcmStorage + ?Sized,
{
    let mut best_tip = *cur_tip;
    let mut best_block = storage
        .get_ol_block(best_tip)
        .await?
        .ok_or(Error::MissingOLBlock(best_tip))?;

    // The implementation of this will only switch to a new tip if it's a higher
    // height than our current tip.  We'll make this more sophisticated in the
    // future if we have a more sophisticated consensus protocol.
    for other_tip in tips {
        if other_tip == cur_tip {
            continue;
        }

        let other_block = storage
            .get_ol_block(*other_tip)
            .await?
            .ok_or(Error::MissingOLBlock(*other_tip))?;

        let best_header = best_block.header();
        let other_header = other_block.header();

        if other_header.slot() > best_header.slot() {
            best_tip = *other_tip;
            best_block = other_block;
        }
    }

    Ok(best_tip)
}

async fn apply_tip_update<C: FcmContext>(
    update: TipUpdate,
    fcm_state: &mut FcmServiceState<C>,
    bundle: &OLBlock,
) -> anyhow::Result<()> {
    match update {
        // Easy case.
        TipUpdate::ExtendTip(_cur, _new) => {
            // TODO(STR-3673): what's the relation between _new and bundle
            // Update the tip block in the FCM state.
            let slot = bundle.header().slot();
            let blkid = bundle.header().compute_blkid();
            let blk_cmmt = OLBlockCommitment::new(slot, blkid);
            let ol_state = fcm_state
                .ctx()
                .get_toplevel_ol_state(blk_cmmt)
                .await?
                .ok_or(Error::MissingOLState(blk_cmmt))?;

            // Capture the old tip slot before the update; it is the truncation pivot.
            let pivot_slot = fcm_state.cur_best_block().slot();
            fcm_state.update_tip_block(blk_cmmt, ol_state).await?;

            record_canonical_suffix(fcm_state, pivot_slot, vec![blkid]).await?;

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

            let pivot_slot = fcm_state.cur_best_block().slot();
            let mut applied = Vec::with_capacity(intermediate.len());
            for blkid in intermediate {
                advance_fcm_to_block(blkid, fcm_state).await?;
                applied.push(blkid);
            }

            let final_tip = fcm_state.cur_best_block();
            let expected_slot = fcm_state.get_block_slot(new).await?;
            let expected_tip = OLBlockCommitment::new(expected_slot, new);
            if final_tip != expected_tip {
                return Err(Error::OLApplyTipMismatch(expected_tip, final_tip).into());
            }

            record_canonical_suffix(fcm_state, pivot_slot, applied).await?;

            Ok(())
        }

        TipUpdate::Reorg(reorg) => {
            // See if we need to roll back recent changes.
            let pivot_blkid = *reorg.pivot();
            let pivot_slot = fcm_state.get_block_slot(pivot_blkid).await?;
            let pivot_block = OLBlockCommitment::new(pivot_slot, pivot_blkid);
            let cur_best = fcm_state.cur_best_block();
            let reorg_depth = reorg.revert_iter().count();
            let reverts_blocks = reorg_depth > 0;

            // We probably need to roll back to an earlier block and update our
            // in-memory state first.
            if reverts_blocks {
                if pivot_slot >= cur_best.slot() {
                    return Err(Error::InvalidOLReorgPivot(pivot_block, cur_best).into());
                }

                debug!(%pivot_blkid, %pivot_slot, "rolling back ol_state");
                revert_ol_state_to_block(&pivot_block, fcm_state).await?;
            } else if pivot_blkid != *cur_best.blkid() {
                return Err(Error::InvalidOLReorgEmptyDownPivot(cur_best, pivot_block).into());
            }

            let mut applied = Vec::new();
            for blkid in reorg.apply_iter().copied() {
                advance_fcm_to_block(blkid, fcm_state).await?;
                applied.push(blkid);
            }

            let final_tip = fcm_state.cur_best_block();
            let expected_tip = if *reorg.new_tip() == pivot_blkid {
                pivot_block
            } else {
                let expected_slot = fcm_state.get_block_slot(*reorg.new_tip()).await?;
                OLBlockCommitment::new(expected_slot, *reorg.new_tip())
            };
            if final_tip != expected_tip {
                return Err(Error::OLApplyTipMismatch(expected_tip, final_tip).into());
            }

            // Truncate the abandoned branch above the pivot and write the new one.
            record_canonical_suffix(fcm_state, pivot_slot, applied).await?;

            counter!("strata_fcm_reorgs_total").increment(1);
            histogram!("strata_fcm_reorg_depth").record(reorg_depth as f64);

            Ok(())
        }

        TipUpdate::Revert(_cur, new) => {
            let slot = fcm_state.get_block_slot(new).await?;
            let block = OLBlockCommitment::new(slot, new);
            revert_ol_state_to_block(&block, fcm_state).await?;

            // Revert to a lower tip; truncate everything above it, write nothing.
            record_canonical_suffix(fcm_state, slot, Vec::new()).await?;

            Ok(())
        }
    }
}

/// Single canonical write path for fork-choice tip moves.
async fn record_canonical_suffix<C: FcmContext>(
    fcm_state: &FcmServiceState<C>,
    pivot_slot: Slot,
    block_ids: Vec<OLBlockId>,
) -> anyhow::Result<()> {
    let Some(start_slot) = pivot_slot.checked_add(1) else {
        // Truncating above the maximum slot is a no-op; only a non-empty suffix is impossible.
        if block_ids.is_empty() {
            return Ok(());
        }
        return Err(Error::FcmCanonicalSuffixAboveMaxSlot(pivot_slot).into());
    };
    fcm_state
        .ctx()
        .replace_canonical_suffix_from(start_slot, block_ids)
        .await?;
    Ok(())
}

/// Advances the in-memory OL state to an already-executed block.
async fn advance_fcm_to_block<C: FcmContext>(
    blkid: OLBlockId,
    fcm_state: &mut FcmServiceState<C>,
) -> anyhow::Result<()> {
    let block = fcm_state
        .ctx()
        .get_ol_block(blkid)
        .await?
        .ok_or(Error::MissingOLBlock(blkid))?;

    let block_commitment = OLBlockCommitment::new(block.header().slot(), blkid);
    let cur_best = fcm_state.cur_best_block();
    let actual_parent = *block.header().parent_blkid();
    if actual_parent != *cur_best.blkid() {
        return Err(
            Error::OLApplyBlockParentMismatch(block_commitment, cur_best, actual_parent).into(),
        );
    }

    let ol_state = fcm_state
        .ctx()
        .get_toplevel_ol_state(block_commitment)
        .await?
        .ok_or(Error::MissingOLState(block_commitment))?;

    fcm_state
        .update_tip_block(block_commitment, ol_state)
        .await?;

    Ok(())
}

/// Safely reverts the in-memory ol_state to a particular block, then rolls
/// back the writes on-disk.
async fn revert_ol_state_to_block<C: FcmContext>(
    block: &OLBlockCommitment,
    fcm_state: &mut FcmServiceState<C>,
) -> anyhow::Result<()> {
    // Fetch the old state from the database and store in memory.  This
    // is also how  we validate that we actually *can* revert to this
    // block.
    let new_state = fcm_state
        .ctx()
        .get_toplevel_ol_state(*block)
        .await?
        .ok_or(Error::MissingOLState(*block))?;
    fcm_state.update_tip_block(*block, new_state).await?;

    // FIXME(STR-2140): Rollback the writes on the database that we no longer need.

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{BTreeMap, HashMap},
        sync::{Arc, Mutex},
    };

    use async_trait::async_trait;
    use strata_asm_common::AsmManifest;
    use strata_db_types::{traits::BlockStatus, DbResult};
    use strata_identifiers::{Epoch, Slot, WtxidsRoot};
    use strata_ol_chain_types_new::{
        test_utils::{schnorr_predicate, test_schnorr_keypair},
        BlockFlags, OLBlock, OLBlockBody, OLBlockCredential, OLBlockHeader, OLTxSegment,
        SignedOLBlockHeader,
    };
    use strata_ol_state_support_types::MemoryStateBaseLayer;
    use strata_ol_state_types::{OLAccountState, OLState, WriteBatch};
    use strata_ol_stf::{
        test_utils::{execute_block, make_genesis_state},
        BlockComponents, BlockInfo, CompletedBlock,
    };
    use strata_predicate::PredicateKey;
    use strata_primitives::{crypto::sign_schnorr_sig, l1::L1BlockId, Buf64, OLBlockId};

    use super::*;
    use crate::{
        fcm::{
            context::{ChainController, CsmStatusReader},
            state::{reconcile_canonical_blocks_index, FcmInnerState},
        },
        tip_update::TipUpdate,
        unfinalized_tracker::{UnfinalizedBlockTracker, UnfinalizedOLBlockSource},
    };

    #[derive(Default)]
    struct StubFcmStorage {
        inner: Mutex<StubFcmStorageInner>,
    }

    #[derive(Default)]
    struct StubFcmStorageInner {
        blocks: HashMap<OLBlockId, OLBlock>,
        statuses: HashMap<OLBlockId, BlockStatus>,
        blocks_by_slot: BTreeMap<Slot, Vec<OLBlockId>>,
        canonical_blocks: HashMap<Slot, OLBlockCommitment>,
        block_high_watermark: Option<OLBlockCommitment>,
        states: HashMap<OLBlockCommitment, Arc<OLState>>,
        canonical_epochs: HashMap<Epoch, EpochCommitment>,
        indexing_rollbacks: Vec<(Epoch, OLBlockCommitment)>,
        epoch_summary_deletes: Vec<EpochCommitment>,
    }

    impl StubFcmStorage {
        fn new() -> Self {
            Self::default()
        }

        fn put_ol_block(&self, block: OLBlock) -> OLBlockCommitment {
            self.put_block_parts(block, None, None)
        }

        fn put_executed_block(
            &self,
            block: OLBlock,
            state: OLState,
            status: BlockStatus,
        ) -> OLBlockCommitment {
            self.put_block_parts(block, Some(state), Some(status))
        }

        fn put_block_parts(
            &self,
            block: OLBlock,
            state: Option<OLState>,
            status: Option<BlockStatus>,
        ) -> OLBlockCommitment {
            let blkid = block.header().compute_blkid();
            let slot = block.header().slot();
            let commitment = OLBlockCommitment::new(slot, blkid);
            let mut inner = self.inner.lock().unwrap();

            inner.blocks.insert(blkid, block);
            let blocks_at_slot = inner.blocks_by_slot.entry(slot).or_default();
            if !blocks_at_slot.contains(&blkid) {
                blocks_at_slot.push(blkid);
            }
            if let Some(state) = state {
                inner.states.insert(commitment, Arc::new(state));
            }
            if let Some(status) = status {
                inner.statuses.insert(blkid, status);
            }

            commitment
        }

        fn put_toplevel_ol_state(&self, block: OLBlockCommitment, state: OLState) {
            let mut inner = self.inner.lock().unwrap();
            assert!(
                inner.blocks.contains_key(block.blkid()),
                "cannot seed OL state without the corresponding OL block"
            );
            inner.states.insert(block, Arc::new(state));
        }

        fn put_canonical_epoch_commitment(&self, epoch: EpochCommitment) {
            self.inner
                .lock()
                .unwrap()
                .canonical_epochs
                .insert(epoch.epoch(), epoch);
        }

        /// Seeds the canonical slot-0 entry, mirroring what real genesis does.
        /// Plain block puts do not touch the canonical index, so tests that boot
        /// FCM must seed slot 0 explicitly or the startup loop never resolves.
        fn seed_canonical_genesis(&self, genesis_blkid: OLBlockId) {
            self.inner
                .lock()
                .unwrap()
                .canonical_blocks
                .insert(0, OLBlockCommitment::new(0, genesis_blkid));
        }

        fn set_block_high_watermark(&self, block: OLBlockCommitment) {
            self.inner.lock().unwrap().block_high_watermark = Some(block);
        }

        fn block_high_watermark(&self) -> Option<OLBlockCommitment> {
            self.inner.lock().unwrap().block_high_watermark
        }

        fn indexing_rollbacks(&self) -> Vec<(Epoch, OLBlockCommitment)> {
            self.inner.lock().unwrap().indexing_rollbacks.clone()
        }

        fn epoch_summary_deletes(&self) -> Vec<EpochCommitment> {
            self.inner.lock().unwrap().epoch_summary_deletes.clone()
        }
    }

    #[derive(Default)]
    struct StubFcmContext {
        storage: StubFcmStorage,
        last_finalized_epoch: Option<EpochCommitment>,
        last_confirmed_epoch: Option<EpochCommitment>,
        executed_blocks: Mutex<Vec<OLBlockCommitment>>,
        safe_tip_updates: Mutex<Vec<OLBlockCommitment>>,
        finalized_epochs: Mutex<Vec<EpochCommitment>>,
        published_statuses: Mutex<Vec<OLSyncStatus>>,
    }

    impl StubFcmContext {
        fn new() -> Self {
            Self::default()
        }

        fn storage(&self) -> &StubFcmStorage {
            &self.storage
        }

        fn with_last_finalized_epoch(mut self, epoch: Option<EpochCommitment>) -> Self {
            self.last_finalized_epoch = epoch;
            self
        }

        fn with_last_confirmed_epoch(mut self, epoch: Option<EpochCommitment>) -> Self {
            self.last_confirmed_epoch = epoch;
            self
        }

        fn executed_blocks(&self) -> Vec<OLBlockCommitment> {
            self.executed_blocks.lock().unwrap().clone()
        }

        fn safe_tip_updates(&self) -> Vec<OLBlockCommitment> {
            self.safe_tip_updates.lock().unwrap().clone()
        }

        fn finalized_epochs(&self) -> Vec<EpochCommitment> {
            self.finalized_epochs.lock().unwrap().clone()
        }

        fn published_statuses(&self) -> Vec<OLSyncStatus> {
            self.published_statuses.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl UnfinalizedOLBlockSource for StubFcmStorage {
        async fn get_blocks_at_height(&self, slot: Slot) -> DbResult<Vec<OLBlockId>> {
            Ok(self
                .inner
                .lock()
                .unwrap()
                .blocks_by_slot
                .get(&slot)
                .cloned()
                .unwrap_or_default())
        }

        async fn get_block_status(&self, blkid: OLBlockId) -> DbResult<Option<BlockStatus>> {
            Ok(self.inner.lock().unwrap().statuses.get(&blkid).copied())
        }

        async fn get_ol_block(&self, blkid: OLBlockId) -> DbResult<Option<OLBlock>> {
            Ok(self.inner.lock().unwrap().blocks.get(&blkid).cloned())
        }
    }

    #[async_trait]
    impl FcmStorage for StubFcmStorage {
        async fn set_block_status(&self, blkid: OLBlockId, status: BlockStatus) -> DbResult<bool> {
            let mut inner = self.inner.lock().unwrap();
            let block_exists = inner.blocks.contains_key(&blkid);
            if block_exists {
                inner.statuses.insert(blkid, status);
            }
            Ok(block_exists)
        }

        async fn clear_block_high_watermark(&self, expected: OLBlockCommitment) -> DbResult<bool> {
            let mut inner = self.inner.lock().unwrap();
            if inner.block_high_watermark != Some(expected) {
                return Ok(false);
            }

            inner.block_high_watermark = None;
            Ok(true)
        }

        async fn get_block_high_watermark(&self) -> DbResult<Option<OLBlockCommitment>> {
            Ok(self.inner.lock().unwrap().block_high_watermark)
        }

        async fn rollback_block_state_indexing(
            &self,
            epoch: Epoch,
            cutoff: OLBlockCommitment,
        ) -> DbResult<()> {
            self.inner
                .lock()
                .unwrap()
                .indexing_rollbacks
                .push((epoch, cutoff));
            Ok(())
        }

        async fn del_epoch_summary(&self, epoch: EpochCommitment) -> DbResult<bool> {
            self.inner.lock().unwrap().epoch_summary_deletes.push(epoch);
            Ok(false)
        }

        async fn get_toplevel_ol_state(
            &self,
            commitment: OLBlockCommitment,
        ) -> DbResult<Option<Arc<OLState>>> {
            Ok(self.inner.lock().unwrap().states.get(&commitment).cloned())
        }

        async fn get_canonical_block_at(&self, slot: Slot) -> DbResult<Option<OLBlockCommitment>> {
            Ok(self
                .inner
                .lock()
                .unwrap()
                .canonical_blocks
                .get(&slot)
                .copied())
        }

        async fn replace_canonical_suffix_from(
            &self,
            start_slot: Slot,
            block_ids: Vec<OLBlockId>,
        ) -> DbResult<()> {
            let mut inner = self.inner.lock().unwrap();
            inner.canonical_blocks.retain(|slot, _| *slot < start_slot);
            let block_count = block_ids.len();
            for (offset, id) in block_ids.into_iter().enumerate() {
                let offset = u64::try_from(offset).map_err(|_| {
                    strata_db_types::DbError::OLCanonicalSuffixOverflow {
                        start_slot,
                        block_count,
                    }
                })?;
                let slot = start_slot.checked_add(offset).ok_or({
                    strata_db_types::DbError::OLCanonicalSuffixOverflow {
                        start_slot,
                        block_count,
                    }
                })?;
                inner
                    .canonical_blocks
                    .insert(slot, OLBlockCommitment::new(slot, id));
            }
            Ok(())
        }

        async fn get_canonical_epoch_commitment_at(
            &self,
            epoch: Epoch,
        ) -> DbResult<Option<EpochCommitment>> {
            Ok(self
                .inner
                .lock()
                .unwrap()
                .canonical_epochs
                .get(&epoch)
                .copied())
        }
    }

    #[async_trait]
    impl ChainController for StubFcmContext {
        async fn try_exec_block(&self, block: OLBlockCommitment) -> anyhow::Result<()> {
            self.executed_blocks.lock().unwrap().push(block);
            Ok(())
        }

        async fn update_safe_tip(&self, safe_tip: OLBlockCommitment) -> anyhow::Result<()> {
            self.safe_tip_updates.lock().unwrap().push(safe_tip);
            Ok(())
        }

        async fn finalize_epoch(&self, epoch: EpochCommitment) -> anyhow::Result<()> {
            self.finalized_epochs.lock().unwrap().push(epoch);
            Ok(())
        }
    }

    impl CsmStatusReader for StubFcmContext {
        fn last_finalized_epoch(&self) -> Option<EpochCommitment> {
            self.last_finalized_epoch
        }

        fn last_confirmed_epoch(&self) -> Option<EpochCommitment> {
            self.last_confirmed_epoch
        }
    }

    #[async_trait]
    impl UnfinalizedOLBlockSource for StubFcmContext {
        async fn get_blocks_at_height(&self, slot: Slot) -> DbResult<Vec<OLBlockId>> {
            self.storage.get_blocks_at_height(slot).await
        }

        async fn get_block_status(&self, blkid: OLBlockId) -> DbResult<Option<BlockStatus>> {
            self.storage.get_block_status(blkid).await
        }

        async fn get_ol_block(&self, blkid: OLBlockId) -> DbResult<Option<OLBlock>> {
            self.storage.get_ol_block(blkid).await
        }
    }

    #[async_trait]
    impl FcmStorage for StubFcmContext {
        async fn set_block_status(&self, blkid: OLBlockId, status: BlockStatus) -> DbResult<bool> {
            self.storage.set_block_status(blkid, status).await
        }

        async fn clear_block_high_watermark(&self, expected: OLBlockCommitment) -> DbResult<bool> {
            self.storage.clear_block_high_watermark(expected).await
        }

        async fn get_block_high_watermark(&self) -> DbResult<Option<OLBlockCommitment>> {
            self.storage.get_block_high_watermark().await
        }

        async fn rollback_block_state_indexing(
            &self,
            epoch: Epoch,
            cutoff: OLBlockCommitment,
        ) -> DbResult<()> {
            self.storage
                .rollback_block_state_indexing(epoch, cutoff)
                .await
        }

        async fn del_epoch_summary(&self, epoch: EpochCommitment) -> DbResult<bool> {
            self.storage.del_epoch_summary(epoch).await
        }

        async fn get_toplevel_ol_state(
            &self,
            commitment: OLBlockCommitment,
        ) -> DbResult<Option<Arc<OLState>>> {
            self.storage.get_toplevel_ol_state(commitment).await
        }

        async fn get_canonical_block_at(&self, slot: Slot) -> DbResult<Option<OLBlockCommitment>> {
            self.storage.get_canonical_block_at(slot).await
        }

        async fn replace_canonical_suffix_from(
            &self,
            start_slot: Slot,
            block_ids: Vec<OLBlockId>,
        ) -> DbResult<()> {
            self.storage
                .replace_canonical_suffix_from(start_slot, block_ids)
                .await
        }

        async fn get_canonical_epoch_commitment_at(
            &self,
            epoch: Epoch,
        ) -> DbResult<Option<EpochCommitment>> {
            self.storage.get_canonical_epoch_commitment_at(epoch).await
        }
    }

    impl FcmContext for StubFcmContext {
        fn publish_sync_status(&self, status: OLSyncStatus) {
            self.published_statuses.lock().unwrap().push(status);
        }
    }

    #[derive(Clone)]
    struct ExecutedBlock {
        block: OLBlock,
        state: OLState,
    }

    impl ExecutedBlock {
        fn new(completed: CompletedBlock, state: &MemoryStateBaseLayer) -> Self {
            let signed_header = SignedOLBlockHeader::new(completed.header().clone(), Buf64::zero());
            Self {
                block: OLBlock::new(signed_header, completed.body().clone()),
                state: state.state().clone(),
            }
        }

        fn blkid(&self) -> OLBlockId {
            self.block.header().compute_blkid()
        }

        fn commitment(&self) -> OLBlockCommitment {
            OLBlockCommitment::new(self.block.header().slot(), self.blkid())
        }
    }

    struct FcmTestFixture {
        ctx: Arc<StubFcmContext>,
    }

    impl FcmTestFixture {
        fn new(genesis: &ExecutedBlock, common_blocks: &[&ExecutedBlock]) -> Self {
            let genesis_epoch = EpochCommitment::new(0, 0, genesis.blkid());
            let ctx = StubFcmContext::new()
                .with_last_finalized_epoch(Some(genesis_epoch))
                .with_last_confirmed_epoch(Some(genesis_epoch));

            seed_executed_block(ctx.storage(), genesis, BlockStatus::Valid);
            for block in common_blocks {
                seed_executed_block(ctx.storage(), block, BlockStatus::Valid);
            }
            ctx.storage().put_canonical_epoch_commitment(genesis_epoch);
            ctx.storage().seed_canonical_genesis(genesis.blkid());

            Self { ctx: Arc::new(ctx) }
        }

        fn fcm_state_at(
            &self,
            tracker: UnfinalizedBlockTracker,
            cur_block: &ExecutedBlock,
        ) -> FcmServiceState<StubFcmContext> {
            let inner = FcmInnerState::new(
                tracker,
                cur_block.commitment(),
                Arc::new(cur_block.state.clone()),
                Vec::new(),
            );
            FcmServiceState::new(self.ctx.clone(), PredicateKey::always_accept(), inner)
        }
    }

    fn execute_test_genesis() -> (ExecutedBlock, MemoryStateBaseLayer) {
        let mut genesis_state = make_genesis_state();
        let genesis_manifest = AsmManifest::new(
            1,
            L1BlockId::from(Buf32::zero()),
            WtxidsRoot::from(Buf32::zero()),
            vec![],
        )
        .expect("valid genesis manifest");
        let genesis_completed = execute_block(
            &mut genesis_state,
            &BlockInfo::new_genesis(1_000),
            None,
            BlockComponents::new_manifests(vec![genesis_manifest]).as_terminal(),
        )
        .expect("genesis executes");
        let genesis = ExecutedBlock::new(genesis_completed, &genesis_state);

        (genesis, genesis_state)
    }

    fn execute_test_block(
        state: &mut MemoryStateBaseLayer,
        parent: &OLBlock,
        timestamp: u64,
        slot: u64,
    ) -> ExecutedBlock {
        execute_test_block_in_epoch(state, parent, timestamp, slot, 1)
    }

    fn execute_test_block_in_epoch(
        state: &mut MemoryStateBaseLayer,
        parent: &OLBlock,
        timestamp: u64,
        slot: u64,
        epoch: Epoch,
    ) -> ExecutedBlock {
        let completed = execute_block(
            state,
            &BlockInfo::new(timestamp, slot, epoch),
            Some(parent.header()),
            BlockComponents::new_empty(),
        )
        .expect("test block executes");

        ExecutedBlock::new(completed, state)
    }

    fn empty_tracker(genesis: &ExecutedBlock) -> UnfinalizedBlockTracker {
        let finalized_epoch = EpochCommitment::new(0, 0, genesis.blkid());
        UnfinalizedBlockTracker::new_empty(finalized_epoch)
    }

    fn attach_test_block(tracker: &mut UnfinalizedBlockTracker, block: &ExecutedBlock) {
        tracker
            .attach_block(
                block.block.header().slot(),
                block.blkid(),
                *block.block.header().parent_blkid(),
            )
            .expect("block attaches to test tracker");
    }

    fn tracker_with_blocks(
        genesis: &ExecutedBlock,
        blocks: &[&ExecutedBlock],
    ) -> UnfinalizedBlockTracker {
        let mut tracker = empty_tracker(genesis);
        for block in blocks {
            attach_test_block(&mut tracker, block);
        }
        tracker
    }

    fn expected_tip_update(
        from: &ExecutedBlock,
        to: &ExecutedBlock,
        tracker: &UnfinalizedBlockTracker,
    ) -> anyhow::Result<TipUpdate> {
        Ok(
            compute_tip_update(&from.blkid(), &to.blkid(), 100, tracker)?
                .expect("test chain should produce a tip update"),
        )
    }

    fn seed_executed_block(
        storage: &StubFcmStorage,
        executed: &ExecutedBlock,
        status: BlockStatus,
    ) {
        storage.put_executed_block(executed.block.clone(), executed.state.clone(), status);
    }

    struct TestFork {
        genesis: ExecutedBlock,
        a1: ExecutedBlock,
        a2: ExecutedBlock,
        b1: ExecutedBlock,
        b2: ExecutedBlock,
        b3: ExecutedBlock,
    }

    impl TestFork {
        fn new() -> Self {
            let (genesis, genesis_state) = execute_test_genesis();

            // Distinct branch timestamps make same-slot A/B blocks produce different block IDs.
            let mut a_state = genesis_state.clone();
            let a1 = execute_test_block(&mut a_state, &genesis.block, 1_100, 1);
            let a2 = execute_test_block(&mut a_state, &a1.block, 1_200, 2);

            let mut b_state = genesis_state;
            let b1 = execute_test_block(&mut b_state, &genesis.block, 2_100, 1);
            let b2 = execute_test_block(&mut b_state, &b1.block, 2_200, 2);
            let b3 = execute_test_block(&mut b_state, &b2.block, 2_300, 3);

            Self {
                genesis,
                a1,
                a2,
                b1,
                b2,
                b3,
            }
        }

        fn fixture(&self) -> FcmTestFixture {
            let common_blocks = [&self.a1, &self.a2, &self.b1, &self.b2];
            FcmTestFixture::new(&self.genesis, &common_blocks)
        }

        fn tracker_without_b3(&self) -> UnfinalizedBlockTracker {
            tracker_with_blocks(&self.genesis, &[&self.a1, &self.a2, &self.b1, &self.b2])
        }

        fn tracker_with_b3(&self) -> UnfinalizedBlockTracker {
            let mut tracker = self.tracker_without_b3();
            attach_test_block(&mut tracker, &self.b3);
            tracker
        }

        fn tracker_with_a1_b1(&self) -> UnfinalizedBlockTracker {
            tracker_with_blocks(&self.genesis, &[&self.a1, &self.b1])
        }
    }

    /// Fixed linear chain used by LongExtend tests.
    struct LinearChain {
        genesis: ExecutedBlock,
        x1: ExecutedBlock,
        x2: ExecutedBlock,
        x3: ExecutedBlock,
        x4: ExecutedBlock,
    }

    impl LinearChain {
        fn new() -> Self {
            let (genesis, mut state) = execute_test_genesis();
            let x1 = execute_test_block(&mut state, &genesis.block, 3_100, 1);
            let x2 = execute_test_block(&mut state, &x1.block, 3_200, 2);
            let x3 = execute_test_block(&mut state, &x2.block, 3_300, 3);
            let x4 = execute_test_block(&mut state, &x3.block, 3_400, 4);

            Self {
                genesis,
                x1,
                x2,
                x3,
                x4,
            }
        }

        fn fixture_without_x4(&self) -> FcmTestFixture {
            let common_blocks = [&self.x1, &self.x2, &self.x3];
            FcmTestFixture::new(&self.genesis, &common_blocks)
        }

        fn tracker_through_x3(&self) -> UnfinalizedBlockTracker {
            tracker_with_blocks(&self.genesis, &[&self.x1, &self.x2, &self.x3])
        }

        fn tracker_through_x4(&self) -> UnfinalizedBlockTracker {
            tracker_with_blocks(&self.genesis, &[&self.x1, &self.x2, &self.x3, &self.x4])
        }
    }

    #[test]
    fn record_observed_finalized_epoch_classifies_ordering() {
        let chain = LinearChain::new();
        let fixture = chain.fixture_without_x4();
        let finalized_epoch =
            EpochCommitment::new(1, chain.x1.commitment().slot(), chain.x1.blkid());
        let tracker = UnfinalizedBlockTracker::new_empty(finalized_epoch);
        let mut fcm_state = fixture.fcm_state_at(tracker, &chain.x1);

        assert!(!fcm_state.record_observed_finalized_epoch(finalized_epoch));

        let strict_regression = EpochCommitment::new(0, 0, chain.genesis.blkid());
        assert!(!fcm_state.record_observed_finalized_epoch(strict_regression));

        let epoch_up_slot_flat =
            EpochCommitment::new(2, finalized_epoch.last_slot(), chain.x2.blkid());
        assert!(!fcm_state.record_observed_finalized_epoch(epoch_up_slot_flat));

        let slot_up_epoch_flat = EpochCommitment::new(
            finalized_epoch.epoch(),
            chain.x2.commitment().slot(),
            chain.x2.blkid(),
        );
        assert!(!fcm_state.record_observed_finalized_epoch(slot_up_epoch_flat));

        let strict_advance =
            EpochCommitment::new(2, chain.x2.commitment().slot(), chain.x2.blkid());
        assert!(fcm_state.record_observed_finalized_epoch(strict_advance));
        assert_eq!(*fcm_state.latest_observed_finalized_epoch(), strict_advance);
    }

    #[tokio::test]
    async fn handle_new_state_update_ignores_repeated_finalized_epoch() -> anyhow::Result<()> {
        let (genesis, _) = execute_test_genesis();
        let genesis_epoch = EpochCommitment::new(0, 0, genesis.blkid());
        let ctx = Arc::new(
            StubFcmContext::new()
                .with_last_finalized_epoch(Some(genesis_epoch))
                .with_last_confirmed_epoch(Some(genesis_epoch)),
        );
        seed_executed_block(ctx.storage(), &genesis, BlockStatus::Valid);
        ctx.storage().put_canonical_epoch_commitment(genesis_epoch);
        ctx.storage().seed_canonical_genesis(genesis.blkid());

        let tracker = empty_tracker(&genesis);
        let inner = FcmInnerState::new(
            tracker,
            genesis.commitment(),
            Arc::new(genesis.state.clone()),
            Vec::new(),
        );
        let mut fcm_state = FcmServiceState::new(ctx.clone(), PredicateKey::always_accept(), inner);

        handle_new_state_update(&mut fcm_state).await?;

        assert!(ctx.finalized_epochs().is_empty());
        assert_eq!(*fcm_state.latest_observed_finalized_epoch(), genesis_epoch);

        Ok(())
    }

    #[tokio::test]
    async fn handle_new_state_update_retries_pending_finalized_epoch() -> anyhow::Result<()> {
        let chain = LinearChain::new();
        let pending_epoch = EpochCommitment::new(1, chain.x1.commitment().slot(), chain.x1.blkid());
        let ctx = Arc::new(
            StubFcmContext::new()
                .with_last_finalized_epoch(Some(pending_epoch))
                .with_last_confirmed_epoch(Some(pending_epoch)),
        );
        let tracker = tracker_with_blocks(&chain.genesis, &[&chain.x1, &chain.x2]);
        let mut finalizable_state = chain.x2.state.clone();
        let mut epoch_update = WriteBatch::<OLAccountState>::default();
        epoch_update.epochal_writes_mut().cur_epoch = Some(2);
        finalizable_state
            .apply_write_batch(epoch_update)
            .expect("test epoch update applies");

        let inner = FcmInnerState::new(
            tracker,
            chain.x2.commitment(),
            Arc::new(finalizable_state),
            Vec::new(),
        );
        let mut fcm_state = FcmServiceState::new(ctx.clone(), PredicateKey::always_accept(), inner);
        assert!(fcm_state.record_observed_finalized_epoch(pending_epoch));

        handle_new_state_update(&mut fcm_state).await?;

        assert_eq!(ctx.finalized_epochs(), vec![pending_epoch]);
        assert_eq!(*fcm_state.chain_tracker().finalized_epoch(), pending_epoch);

        Ok(())
    }

    #[tokio::test]
    async fn reorg_applies_up_branch_to_new_tip() -> anyhow::Result<()> {
        let fork = TestFork::new();
        let fixture = fork.fixture();
        seed_executed_block(fixture.ctx.storage(), &fork.b3, BlockStatus::Valid);
        let tracker = fork.tracker_with_b3();
        let mut fcm_state = fixture.fcm_state_at(tracker, &fork.a2);
        let update = expected_tip_update(&fork.a2, &fork.b3, fcm_state.chain_tracker())?;

        apply_tip_update(update, &mut fcm_state, &fork.b3.block).await?;

        assert_eq!(fcm_state.cur_best_block(), fork.b3.commitment());
        assert_eq!(
            fcm_state.cur_ol_state().global_state().get_cur_slot(),
            fork.b3.state.global_state().get_cur_slot()
        );
        assert_eq!(
            fixture.ctx.safe_tip_updates(),
            vec![
                fork.genesis.commitment(),
                fork.b1.commitment(),
                fork.b2.commitment(),
                fork.b3.commitment()
            ]
        );

        Ok(())
    }

    #[tokio::test]
    async fn single_block_reorg_applies_one_up_block() -> anyhow::Result<()> {
        let fork = TestFork::new();
        let fixture = fork.fixture();
        let tracker = fork.tracker_with_a1_b1();
        let mut fcm_state = fixture.fcm_state_at(tracker, &fork.a1);
        let update = expected_tip_update(&fork.a1, &fork.b1, fcm_state.chain_tracker())?;

        apply_tip_update(update, &mut fcm_state, &fork.b1.block).await?;

        assert_eq!(fcm_state.cur_best_block(), fork.b1.commitment());
        assert_eq!(
            fcm_state.cur_ol_state().global_state().get_cur_slot(),
            fork.b1.state.global_state().get_cur_slot()
        );
        assert_eq!(
            fixture.ctx.safe_tip_updates(),
            vec![fork.genesis.commitment(), fork.b1.commitment()]
        );

        Ok(())
    }

    #[tokio::test]
    async fn reorg_rejects_up_block_with_wrong_parent() -> anyhow::Result<()> {
        let fork = TestFork::new();
        let fixture = fork.fixture();
        let mut tracker = tracker_with_blocks(&fork.genesis, &[&fork.a1]);
        tracker
            .attach_block(
                fork.a2.block.header().slot(),
                fork.a2.blkid(),
                fork.genesis.blkid(),
            )
            .expect("test setup should attach A2 with genesis as tracked parent");

        let mut fcm_state = fixture.fcm_state_at(tracker, &fork.a1);
        let update = expected_tip_update(&fork.a1, &fork.a2, fcm_state.chain_tracker())?;

        let err = apply_tip_update(update, &mut fcm_state, &fork.a2.block)
            .await
            .expect_err("storage header parent must match current FCM tip");

        assert!(matches!(
            err.downcast_ref::<Error>(),
            Some(Error::OLApplyBlockParentMismatch(block, expected_parent, got_parent))
                if *block == fork.a2.commitment()
                    && *expected_parent == fork.genesis.commitment()
                    && *got_parent == fork.a1.blkid()
        ));
        assert_eq!(fcm_state.cur_best_block(), fork.genesis.commitment());

        Ok(())
    }

    #[tokio::test]
    async fn reorg_missing_up_state_errors_after_partial_apply() -> anyhow::Result<()> {
        let fork = TestFork::new();
        let fixture = fork.fixture();
        fixture.ctx.storage().put_ol_block(fork.b3.block.clone());
        let tracker = fork.tracker_with_b3();
        let mut fcm_state = fixture.fcm_state_at(tracker, &fork.a2);
        let update = expected_tip_update(&fork.a2, &fork.b3, fcm_state.chain_tracker())?;

        let err = apply_tip_update(update, &mut fcm_state, &fork.b3.block)
            .await
            .expect_err("missing B3 state should fail reorg apply");

        assert!(matches!(
            err.downcast_ref::<Error>(),
            Some(Error::MissingOLState(commitment)) if *commitment == fork.b3.commitment()
        ));
        assert_eq!(
            fcm_state.cur_best_block(),
            fork.b2.commitment(),
            "mid-loop failures bubble without compensating rollback"
        );

        Ok(())
    }

    #[tokio::test]
    async fn process_fc_message_publishes_reorg_new_tip() -> anyhow::Result<()> {
        let fork = TestFork::new();
        let fixture = fork.fixture();
        seed_executed_block(fixture.ctx.storage(), &fork.b3, BlockStatus::Valid);
        let tracker = fork.tracker_without_b3();
        let mut fcm_state = fixture.fcm_state_at(tracker, &fork.a2);

        process_fc_message(
            &ForkChoiceMessage::NewBlock(fork.b3.blkid()),
            &mut fcm_state,
        )
        .await?;

        let statuses = fixture.ctx.published_statuses();
        assert_eq!(statuses.len(), 1);
        let status = &statuses[0];
        assert_eq!(status.tip, fork.b3.commitment());
        assert_eq!(fcm_state.cur_best_block(), fork.b3.commitment());
        assert_eq!(fixture.ctx.executed_blocks(), vec![fork.b3.commitment()]);

        Ok(())
    }

    /// A reorg from branch A to branch B must rewrite the canonical index to the
    /// new branch and drop the abandoned branch's entries, including any slot the
    /// shorter branch no longer reaches.
    #[tokio::test]
    async fn reorg_rewrites_canonical_index_and_drops_abandoned_branch() -> anyhow::Result<()> {
        let fork = TestFork::new();
        let fixture = fork.fixture();
        seed_executed_block(fixture.ctx.storage(), &fork.b3, BlockStatus::Valid);
        let tracker = fork.tracker_without_b3();
        let mut fcm_state = fixture.fcm_state_at(tracker, &fork.a2);

        // Seed the canonical index as if branch A were the live chain.
        fixture
            .ctx
            .storage()
            .replace_canonical_suffix_from(1, vec![fork.a1.blkid(), fork.a2.blkid()])
            .await?;

        process_fc_message(
            &ForkChoiceMessage::NewBlock(fork.b3.blkid()),
            &mut fcm_state,
        )
        .await?;

        let storage = fixture.ctx.storage();
        // Branch B is now canonical at every slot.
        assert_eq!(
            storage.get_canonical_block_at(1).await?,
            Some(fork.b1.commitment())
        );
        assert_eq!(
            storage.get_canonical_block_at(2).await?,
            Some(fork.b2.commitment())
        );
        assert_eq!(
            storage.get_canonical_block_at(3).await?,
            Some(fork.b3.commitment())
        );
        // Branch A's blocks no longer win their slots.
        assert_ne!(
            storage.get_canonical_block_at(1).await?,
            Some(fork.a1.commitment())
        );
        assert_ne!(
            storage.get_canonical_block_at(2).await?,
            Some(fork.a2.commitment())
        );

        Ok(())
    }

    #[tokio::test]
    async fn revert_truncates_canonical_index_above_new_tip() -> anyhow::Result<()> {
        let chain = LinearChain::new();
        let fixture = chain.fixture_without_x4();
        let tracker = chain.tracker_through_x3();
        let mut fcm_state = fixture.fcm_state_at(tracker, &chain.x3);

        let storage = fixture.ctx.storage();
        storage
            .replace_canonical_suffix_from(
                1,
                vec![chain.x1.blkid(), chain.x2.blkid(), chain.x3.blkid()],
            )
            .await?;

        let update = expected_tip_update(&chain.x3, &chain.x1, fcm_state.chain_tracker())?;
        assert!(matches!(update, TipUpdate::Revert(..)));

        apply_tip_update(update, &mut fcm_state, &chain.x1.block).await?;

        assert_eq!(fcm_state.cur_best_block(), chain.x1.commitment());
        assert_eq!(
            storage.get_canonical_block_at(1).await?,
            Some(chain.x1.commitment())
        );
        assert_eq!(storage.get_canonical_block_at(2).await?, None);
        assert_eq!(storage.get_canonical_block_at(3).await?, None);

        Ok(())
    }

    #[tokio::test]
    async fn reconcile_truncates_stale_canonical_entries_on_restart() -> anyhow::Result<()> {
        let chain = LinearChain::new();
        let fixture = chain.fixture_without_x4();
        let tracker = tracker_with_blocks(&chain.genesis, &[&chain.x1]);

        let storage = fixture.ctx.storage();
        // Simulate a crash that left the index pointing past the recovered tip (x1): slots 2 and 3
        // still hold blocks no longer canonical.
        storage
            .replace_canonical_suffix_from(
                1,
                vec![chain.x1.blkid(), chain.x2.blkid(), chain.x3.blkid()],
            )
            .await?;

        reconcile_canonical_blocks_index(&tracker, chain.x1.commitment(), storage).await?;

        assert_eq!(
            storage.get_canonical_block_at(1).await?,
            Some(chain.x1.commitment())
        );
        assert_eq!(storage.get_canonical_block_at(2).await?, None);
        assert_eq!(storage.get_canonical_block_at(3).await?, None);

        Ok(())
    }

    #[tokio::test]
    async fn long_extend_applies_intermediate_blocks_to_new_tip() -> anyhow::Result<()> {
        let chain = LinearChain::new();
        let fixture = chain.fixture_without_x4();
        seed_executed_block(fixture.ctx.storage(), &chain.x4, BlockStatus::Valid);
        let tracker = chain.tracker_through_x4();
        let mut fcm_state = fixture.fcm_state_at(tracker, &chain.x1);
        let update = expected_tip_update(&chain.x1, &chain.x4, fcm_state.chain_tracker())?;

        assert!(matches!(update, TipUpdate::LongExtend(..)));

        apply_tip_update(update, &mut fcm_state, &chain.x4.block).await?;

        assert_eq!(fcm_state.cur_best_block(), chain.x4.commitment());
        assert_eq!(
            fcm_state.cur_ol_state().global_state().get_cur_slot(),
            chain.x4.state.global_state().get_cur_slot()
        );
        assert_eq!(
            fixture.ctx.safe_tip_updates(),
            vec![
                chain.x2.commitment(),
                chain.x3.commitment(),
                chain.x4.commitment()
            ]
        );

        Ok(())
    }

    #[tokio::test]
    async fn process_fc_message_publishes_long_extend_new_tip() -> anyhow::Result<()> {
        let chain = LinearChain::new();
        let fixture = chain.fixture_without_x4();
        seed_executed_block(fixture.ctx.storage(), &chain.x4, BlockStatus::Valid);
        let tracker = chain.tracker_through_x3();
        let mut fcm_state = fixture.fcm_state_at(tracker, &chain.x1);

        process_fc_message(
            &ForkChoiceMessage::NewBlock(chain.x4.blkid()),
            &mut fcm_state,
        )
        .await?;

        let statuses = fixture.ctx.published_statuses();
        assert_eq!(statuses.len(), 1);
        let status = &statuses[0];
        assert_eq!(status.tip, chain.x4.commitment());
        assert_eq!(fcm_state.cur_best_block(), chain.x4.commitment());
        assert_eq!(fixture.ctx.executed_blocks(), vec![chain.x4.commitment()]);
        assert_eq!(
            fixture.ctx.safe_tip_updates(),
            vec![
                chain.x2.commitment(),
                chain.x3.commitment(),
                chain.x4.commitment()
            ]
        );

        Ok(())
    }

    fn make_block(slot: u64, signature: Option<Buf64>) -> OLBlock {
        let body = OLBlockBody::new_common(OLTxSegment::new(vec![]).expect("empty tx segment"));
        let header = OLBlockHeader::new(
            1_000 + slot,
            BlockFlags::from(0),
            slot,
            0,
            OLBlockId::from(Buf32::zero()),
            body.compute_hash_commitment(),
            Buf32::zero(),
            Buf32::zero(),
        );
        let signed_header = match signature {
            Some(signature) => SignedOLBlockHeader::new(header, signature),
            None => SignedOLBlockHeader {
                header,
                credential: OLBlockCredential {
                    schnorr_sig: None::<Buf64>.into(),
                },
            },
        };

        OLBlock::new(signed_header, body)
    }

    fn make_storage_block(slot: Slot, parent: OLBlockId) -> OLBlock {
        let body = OLBlockBody::new_common(OLTxSegment::new(vec![]).expect("empty tx segment"));
        let header = OLBlockHeader::new(
            1_000 + slot,
            BlockFlags::from(0),
            slot,
            0,
            parent,
            body.compute_hash_commitment(),
            Buf32::zero(),
            Buf32::zero(),
        );
        OLBlock::new(SignedOLBlockHeader::new(header, Buf64::zero()), body)
    }

    fn make_terminal_storage_block(slot: Slot, parent: OLBlockId) -> OLBlock {
        let body = OLBlockBody::new_common(OLTxSegment::new(vec![]).expect("empty tx segment"));
        let mut flags = BlockFlags::from(0);
        flags.set_is_terminal(true);
        let header = OLBlockHeader::new(
            1_000 + slot,
            flags,
            slot,
            0,
            parent,
            body.compute_hash_commitment(),
            Buf32::zero(),
            Buf32::zero(),
        );
        OLBlock::new(SignedOLBlockHeader::new(header, Buf64::zero()), body)
    }

    fn sign_block(block: &OLBlock, signing_key: &Buf32) -> Buf64 {
        let msg: Buf32 = block.header().compute_blkid().into();
        sign_schnorr_sig(&msg, signing_key)
    }

    #[tokio::test]
    async fn on_launch_replays_startup_candidates_and_drains_them() -> anyhow::Result<()> {
        let genesis = make_storage_block(0, OLBlockId::from(Buf32::zero()));
        let genesis_blkid = genesis.header().compute_blkid();
        let genesis_commitment = OLBlockCommitment::new(genesis.header().slot(), genesis_blkid);
        let genesis_epoch = EpochCommitment::new(0, genesis_commitment.slot(), genesis_blkid);
        let ctx = Arc::new(
            StubFcmContext::new()
                .with_last_finalized_epoch(Some(genesis_epoch))
                .with_last_confirmed_epoch(Some(genesis_epoch)),
        );

        let block1 = make_storage_block(1, genesis_blkid);
        let blkid1 = block1.header().compute_blkid();
        let commitment1 = OLBlockCommitment::new(block1.header().slot(), blkid1);
        let block2 = make_storage_block(2, blkid1);
        let blkid2 = block2.header().compute_blkid();
        let commitment2 = OLBlockCommitment::new(block2.header().slot(), blkid2);

        ctx.storage().put_executed_block(
            genesis,
            make_genesis_state().state().clone(),
            BlockStatus::Valid,
        );
        for (block, commitment) in [(block1, commitment1), (block2, commitment2)] {
            let blkid = block.header().compute_blkid();
            ctx.storage().put_ol_block(block);
            ctx.storage()
                .set_block_status(blkid, BlockStatus::Unchecked)
                .await?;
            ctx.storage()
                .put_toplevel_ol_state(commitment, make_genesis_state().state().clone());
        }
        ctx.storage().put_canonical_epoch_commitment(genesis_epoch);
        ctx.storage().seed_canonical_genesis(genesis_blkid);

        let mut fcm_state =
            init_fcm_service_state(PredicateKey::always_accept(), ctx.clone()).await?;

        <FcmService<StubFcmContext> as AsyncService>::on_launch(&mut fcm_state).await?;

        assert_eq!(ctx.executed_blocks(), vec![commitment1, commitment2]);
        assert_eq!(ctx.safe_tip_updates(), vec![commitment1, commitment2]);
        assert_eq!(fcm_state.cur_best_block(), commitment2);
        assert_eq!(fcm_state.take_startup_replay_candidates(), Vec::new());
        assert_eq!(
            ctx.storage().get_block_status(blkid1).await?,
            Some(BlockStatus::Valid)
        );
        assert_eq!(
            ctx.storage().get_block_status(blkid2).await?,
            Some(BlockStatus::Valid)
        );

        Ok(())
    }

    #[tokio::test]
    async fn stub_storage_round_trips_blocks_and_statuses() {
        let storage = StubFcmStorage::new();
        let block = make_storage_block(1, OLBlockId::from(Buf32::zero()));
        let blkid = block.header().compute_blkid();
        let commitment = OLBlockCommitment::new(block.header().slot(), blkid);

        storage.put_ol_block(block.clone());

        assert_eq!(storage.get_ol_block(blkid).await.unwrap(), Some(block));
        assert_eq!(storage.get_blocks_at_height(1).await.unwrap(), vec![blkid]);
        // A plain put does not make a block canonical; that requires an explicit
        // canonical write.
        assert_eq!(storage.get_canonical_block_at(1).await.unwrap(), None);
        storage
            .replace_canonical_suffix_from(1, vec![blkid])
            .await
            .unwrap();
        assert_eq!(
            storage.get_canonical_block_at(1).await.unwrap(),
            Some(commitment)
        );
        assert_eq!(storage.get_block_status(blkid).await.unwrap(), None);

        assert!(storage
            .set_block_status(blkid, BlockStatus::Valid)
            .await
            .unwrap());
        assert_eq!(
            storage.get_block_status(blkid).await.unwrap(),
            Some(BlockStatus::Valid)
        );
    }

    #[tokio::test]
    async fn stub_storage_round_trips_executed_blocks_and_epochs() {
        let storage = StubFcmStorage::new();
        let state = make_genesis_state().state().clone();
        let block = make_storage_block(0, OLBlockId::from(Buf32::zero()));
        let blkid = block.header().compute_blkid();
        let commitment = OLBlockCommitment::new(block.header().slot(), blkid);
        let epoch = EpochCommitment::new(0, commitment.slot(), blkid);

        storage.put_executed_block(block, state, BlockStatus::Valid);
        storage.put_canonical_epoch_commitment(epoch);

        assert!(storage
            .get_toplevel_ol_state(commitment)
            .await
            .unwrap()
            .is_some());
        assert_eq!(
            storage.get_block_status(blkid).await.unwrap(),
            Some(BlockStatus::Valid)
        );
        assert_eq!(
            storage.get_canonical_epoch_commitment_at(0).await.unwrap(),
            Some(epoch)
        );
    }

    #[tokio::test]
    async fn stub_storage_returns_missing_values_without_creating_statuses() {
        let storage = StubFcmStorage::new();
        let missing = OLBlockId::from(Buf32::zero());

        assert_eq!(storage.get_ol_block(missing).await.unwrap(), None);
        assert_eq!(storage.get_blocks_at_height(9).await.unwrap(), Vec::new());
        assert_eq!(storage.get_canonical_block_at(9).await.unwrap(), None);
        assert!(!storage
            .set_block_status(missing, BlockStatus::Invalid)
            .await
            .unwrap());
        assert_eq!(storage.get_block_status(missing).await.unwrap(), None);
    }

    #[tokio::test]
    async fn process_fc_message_uses_stub_context_and_publishes_status() {
        let genesis = make_storage_block(0, OLBlockId::from(Buf32::zero()));
        let genesis_blkid = genesis.header().compute_blkid();
        let genesis_commitment = OLBlockCommitment::new(genesis.header().slot(), genesis_blkid);
        let genesis_epoch = EpochCommitment::new(0, genesis_commitment.slot(), genesis_blkid);
        let ctx = Arc::new(
            StubFcmContext::new()
                .with_last_finalized_epoch(Some(genesis_epoch))
                .with_last_confirmed_epoch(Some(genesis_epoch)),
        );

        let block = make_storage_block(1, genesis_blkid);
        let blkid = block.header().compute_blkid();
        let block_commitment = OLBlockCommitment::new(block.header().slot(), blkid);

        ctx.storage().put_executed_block(
            genesis,
            make_genesis_state().state().clone(),
            BlockStatus::Valid,
        );
        ctx.storage().put_ol_block(block);
        ctx.storage()
            .set_block_status(blkid, BlockStatus::Unchecked)
            .await
            .expect("set unchecked status");
        ctx.storage()
            .put_toplevel_ol_state(block_commitment, make_genesis_state().state().clone());
        ctx.storage().put_canonical_epoch_commitment(genesis_epoch);
        ctx.storage().seed_canonical_genesis(genesis_blkid);

        let mut fcm_state = init_fcm_service_state(PredicateKey::always_accept(), ctx.clone())
            .await
            .expect("FCM state initializes from stub context");
        assert_eq!(fcm_state.take_startup_replay_candidates(), vec![blkid]);

        process_fc_message(&ForkChoiceMessage::NewBlock(blkid), &mut fcm_state)
            .await
            .expect("new block processes through stub context");

        assert_eq!(fcm_state.cur_best_block(), block_commitment);
        assert_eq!(ctx.executed_blocks(), vec![block_commitment]);
        assert_eq!(ctx.safe_tip_updates(), vec![block_commitment]);
        assert_eq!(ctx.finalized_epochs(), Vec::new());
        assert_eq!(
            ctx.storage().get_block_status(blkid).await.unwrap(),
            Some(BlockStatus::Valid)
        );

        let statuses = ctx.published_statuses();
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].tip, block_commitment);
        assert_eq!(statuses[0].prev_epoch, genesis_epoch);
        assert_eq!(statuses[0].confirmed_epoch, genesis_epoch);
        assert_eq!(statuses[0].finalized_epoch, genesis_epoch);
    }

    #[tokio::test]
    async fn process_fc_message_clears_high_watermark_for_invalid_block() -> anyhow::Result<()> {
        let (_, pk) = test_schnorr_keypair();
        let genesis = make_storage_block(0, OLBlockId::from(Buf32::zero()));
        let genesis_blkid = genesis.header().compute_blkid();
        let genesis_commitment = OLBlockCommitment::new(genesis.header().slot(), genesis_blkid);
        let genesis_epoch = EpochCommitment::new(0, genesis_commitment.slot(), genesis_blkid);
        let ctx = Arc::new(
            StubFcmContext::new()
                .with_last_finalized_epoch(Some(genesis_epoch))
                .with_last_confirmed_epoch(Some(genesis_epoch)),
        );

        let block = make_storage_block(1, genesis_blkid);
        let blkid = block.header().compute_blkid();
        let block_commitment = OLBlockCommitment::new(block.header().slot(), blkid);

        ctx.storage().put_executed_block(
            genesis,
            make_genesis_state().state().clone(),
            BlockStatus::Valid,
        );
        ctx.storage().put_ol_block(block);
        ctx.storage()
            .set_block_status(blkid, BlockStatus::Unchecked)
            .await?;
        ctx.storage().set_block_high_watermark(block_commitment);
        ctx.storage().put_canonical_epoch_commitment(genesis_epoch);
        ctx.storage().seed_canonical_genesis(genesis_blkid);

        let mut fcm_state = init_fcm_service_state(schnorr_predicate(&pk), ctx.clone()).await?;
        assert_eq!(fcm_state.take_startup_replay_candidates(), vec![blkid]);

        process_fc_message(&ForkChoiceMessage::NewBlock(blkid), &mut fcm_state).await?;

        assert_eq!(
            ctx.storage().get_block_status(blkid).await?,
            Some(BlockStatus::Invalid)
        );
        assert_eq!(ctx.storage().block_high_watermark(), None);
        // The rejected block's indexing writes are rolled back to its parent,
        // so a replacement block at the same slot can apply its own.
        assert_eq!(
            ctx.storage().indexing_rollbacks(),
            vec![(0, genesis_commitment)]
        );
        // Non-terminal blocks never store a summary, so none is deleted.
        assert!(ctx.storage().epoch_summary_deletes().is_empty());

        Ok(())
    }

    #[tokio::test]
    async fn process_fc_message_deletes_summary_for_invalid_terminal_block() -> anyhow::Result<()> {
        let (_, pk) = test_schnorr_keypair();
        let genesis = make_storage_block(0, OLBlockId::from(Buf32::zero()));
        let genesis_blkid = genesis.header().compute_blkid();
        let genesis_commitment = OLBlockCommitment::new(genesis.header().slot(), genesis_blkid);
        let genesis_epoch = EpochCommitment::new(0, genesis_commitment.slot(), genesis_blkid);
        let ctx = Arc::new(
            StubFcmContext::new()
                .with_last_finalized_epoch(Some(genesis_epoch))
                .with_last_confirmed_epoch(Some(genesis_epoch)),
        );

        let block = make_terminal_storage_block(1, genesis_blkid);
        let blkid = block.header().compute_blkid();
        let block_commitment = OLBlockCommitment::new(block.header().slot(), blkid);

        ctx.storage().put_executed_block(
            genesis,
            make_genesis_state().state().clone(),
            BlockStatus::Valid,
        );
        ctx.storage().put_ol_block(block);
        ctx.storage().set_block_high_watermark(block_commitment);
        ctx.storage().put_canonical_epoch_commitment(genesis_epoch);
        ctx.storage().seed_canonical_genesis(genesis_blkid);

        let mut fcm_state = init_fcm_service_state(schnorr_predicate(&pk), ctx.clone()).await?;

        process_fc_message(&ForkChoiceMessage::NewBlock(blkid), &mut fcm_state).await?;

        assert_eq!(
            ctx.storage().get_block_status(blkid).await?,
            Some(BlockStatus::Invalid)
        );
        assert_eq!(ctx.storage().block_high_watermark(), None);
        assert_eq!(
            ctx.storage().indexing_rollbacks(),
            vec![(0, genesis_commitment)]
        );
        // A rejected terminal block's epoch summary is dropped so it cannot
        // shadow the replacement terminal's summary in canonical lookups.
        assert_eq!(
            ctx.storage().epoch_summary_deletes(),
            vec![EpochCommitment::new(0, 1, blkid)]
        );

        Ok(())
    }

    #[tokio::test]
    async fn process_fc_message_skips_indexing_rollback_for_non_high_watermark_block(
    ) -> anyhow::Result<()> {
        let (_, pk) = test_schnorr_keypair();
        let genesis = make_storage_block(0, OLBlockId::from(Buf32::zero()));
        let genesis_blkid = genesis.header().compute_blkid();
        let genesis_commitment = OLBlockCommitment::new(genesis.header().slot(), genesis_blkid);
        let genesis_epoch = EpochCommitment::new(0, genesis_commitment.slot(), genesis_blkid);
        let ctx = Arc::new(
            StubFcmContext::new()
                .with_last_finalized_epoch(Some(genesis_epoch))
                .with_last_confirmed_epoch(Some(genesis_epoch)),
        );

        // An accepted canonical block holds the high-watermark at slot 1...
        let canonical = make_storage_block(1, genesis_blkid);
        let canonical_commitment = OLBlockCommitment::new(
            canonical.header().slot(),
            canonical.header().compute_blkid(),
        );

        // ...and an unsigned fork block at the same slot arrives afterwards.
        let fork = make_storage_block(1, OLBlockId::from(Buf32::from([7u8; 32])));
        let fork_blkid = fork.header().compute_blkid();
        assert_ne!(fork_blkid, *canonical_commitment.blkid());

        ctx.storage().put_executed_block(
            genesis,
            make_genesis_state().state().clone(),
            BlockStatus::Valid,
        );
        ctx.storage().put_executed_block(
            canonical,
            make_genesis_state().state().clone(),
            BlockStatus::Valid,
        );
        ctx.storage().put_ol_block(fork);
        ctx.storage().set_block_high_watermark(canonical_commitment);
        ctx.storage().put_canonical_epoch_commitment(genesis_epoch);
        ctx.storage().seed_canonical_genesis(genesis_blkid);

        let mut fcm_state = init_fcm_service_state(schnorr_predicate(&pk), ctx.clone()).await?;

        process_fc_message(&ForkChoiceMessage::NewBlock(fork_blkid), &mut fcm_state).await?;

        assert_eq!(
            ctx.storage().get_block_status(fork_blkid).await?,
            Some(BlockStatus::Invalid)
        );
        // The fork block is not the high-watermark: the canonical chain's
        // indexing rows must stay untouched and the high-watermark kept.
        assert_eq!(ctx.storage().indexing_rollbacks(), vec![]);
        assert_eq!(
            ctx.storage().block_high_watermark(),
            Some(canonical_commitment)
        );
        // Non-terminal blocks never store a summary, so none is deleted.
        assert!(ctx.storage().epoch_summary_deletes().is_empty());

        Ok(())
    }

    #[tokio::test]
    async fn process_fc_message_deletes_summary_for_invalid_non_high_watermark_terminal_block(
    ) -> anyhow::Result<()> {
        let (_, pk) = test_schnorr_keypair();
        let genesis = make_storage_block(0, OLBlockId::from(Buf32::zero()));
        let genesis_blkid = genesis.header().compute_blkid();
        let genesis_commitment = OLBlockCommitment::new(genesis.header().slot(), genesis_blkid);
        let genesis_epoch = EpochCommitment::new(0, genesis_commitment.slot(), genesis_blkid);
        let ctx = Arc::new(
            StubFcmContext::new()
                .with_last_finalized_epoch(Some(genesis_epoch))
                .with_last_confirmed_epoch(Some(genesis_epoch)),
        );

        // An accepted canonical block holds the high-watermark at slot 1...
        let canonical = make_storage_block(1, genesis_blkid);
        let canonical_commitment = OLBlockCommitment::new(
            canonical.header().slot(),
            canonical.header().compute_blkid(),
        );

        // ...and an unsigned *terminal* fork block at the same slot arrives.
        let fork = make_terminal_storage_block(1, OLBlockId::from(Buf32::from([7u8; 32])));
        let fork_blkid = fork.header().compute_blkid();

        ctx.storage().put_executed_block(
            genesis,
            make_genesis_state().state().clone(),
            BlockStatus::Valid,
        );
        ctx.storage().put_executed_block(
            canonical,
            make_genesis_state().state().clone(),
            BlockStatus::Valid,
        );
        ctx.storage().put_ol_block(fork);
        ctx.storage().set_block_high_watermark(canonical_commitment);
        ctx.storage().put_canonical_epoch_commitment(genesis_epoch);
        ctx.storage().seed_canonical_genesis(genesis_blkid);

        let mut fcm_state = init_fcm_service_state(schnorr_predicate(&pk), ctx.clone()).await?;

        process_fc_message(&ForkChoiceMessage::NewBlock(fork_blkid), &mut fcm_state).await?;

        assert_eq!(
            ctx.storage().get_block_status(fork_blkid).await?,
            Some(BlockStatus::Invalid)
        );
        // The exact-keyed summary delete runs even though the fork is not the
        // high-watermark: a stale summary keyed by the rejected terminal must
        // not shadow canonical epoch lookups.
        assert_eq!(
            ctx.storage().epoch_summary_deletes(),
            vec![EpochCommitment::new(0, 1, fork_blkid)]
        );
        // The suffix-shaped cleanup stays gated: no indexing rollback and the
        // canonical high-watermark is kept.
        assert_eq!(ctx.storage().indexing_rollbacks(), vec![]);
        assert_eq!(
            ctx.storage().block_high_watermark(),
            Some(canonical_commitment)
        );

        Ok(())
    }

    #[test]
    fn accepts_unsigned_block_when_always_accept() {
        let predicate = PredicateKey::always_accept();
        let block = make_block(1, None);
        let blkid = block.header().compute_blkid();

        let result = check_ol_block_proposal_valid(&blkid, &block, &predicate);

        assert!(result.is_ok());
    }

    #[test]
    fn rejects_unsigned_block_when_checked() {
        let (_, pk) = test_schnorr_keypair();
        let predicate = schnorr_predicate(&pk);
        let block = make_block(1, None);
        let blkid = block.header().compute_blkid();

        let err = check_ol_block_proposal_valid(&blkid, &block, &predicate)
            .expect_err("missing signature should be rejected");

        assert!(matches!(err, Error::MissingBlockSignature(_)));
    }

    #[test]
    fn rejects_invalid_signature() {
        let (_, pk) = test_schnorr_keypair();
        let predicate = schnorr_predicate(&pk);
        let block = make_block(1, Some(Buf64::zero()));
        let blkid = block.header().compute_blkid();

        let err = check_ol_block_proposal_valid(&blkid, &block, &predicate)
            .expect_err("invalid signature should be rejected");

        assert!(matches!(err, Error::InvalidBlockSignature(_)));
    }

    #[test]
    fn accepts_garbage_signature_when_always_accept() {
        let predicate = PredicateKey::always_accept();
        let block = make_block(1, Some(Buf64::zero()));
        let blkid = block.header().compute_blkid();

        let result = check_ol_block_proposal_valid(&blkid, &block, &predicate);

        assert!(result.is_ok());
    }

    #[test]
    fn accepts_valid_signature() {
        let (sk, pk) = test_schnorr_keypair();
        let predicate = schnorr_predicate(&pk);
        let block = make_block(1, Some(Buf64::zero()));
        let signature = sign_block(&block, &sk);
        let block = make_block(1, Some(signature));
        let blkid = block.header().compute_blkid();

        let result = check_ol_block_proposal_valid(&blkid, &block, &predicate);

        assert!(result.is_ok());
    }

    #[test]
    fn rejects_genesis_block_proposal() {
        let (_, pk) = test_schnorr_keypair();
        let predicate = schnorr_predicate(&pk);
        let block = make_block(0, None);
        let blkid = block.header().compute_blkid();

        let err = check_ol_block_proposal_valid(&blkid, &block, &predicate)
            .expect_err("slot-0 proposals should be rejected");

        assert!(matches!(err, Error::UnexpectedGenesisBlock(_)));
    }
}
