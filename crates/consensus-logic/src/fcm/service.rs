use std::{marker::PhantomData, num::NonZeroUsize, sync::Arc};

use anyhow::{anyhow, Context};
use metrics::{counter, histogram};
use serde::Serialize;
use strata_csm_types::CheckpointState;
use strata_db_types::{
    ol_block::{BlockStatus, StatusScanStart},
    DbError,
};
use strata_identifiers::Slot;
use strata_ol_chain_types_v1::{
    sequencer_predicate_requires_signature, verify_sequencer_predicate_signature, OLBlockV1,
};
use strata_predicate::PredicateKey;
use strata_primitives::{Buf32, EpochCommitment, L1BlockCommitment, OLBlockCommitment, OLBlockId};
use strata_service::{AsyncService, Response, Service, ServiceBuilder, ServiceMonitor};
use strata_status::OLSyncStatus;
use strata_tasks::TaskExecutor;
use thiserror::Error as ThisError;
use tokio::sync::{
    mpsc::{channel as mpsc_channel, Sender},
    watch,
};
use tracing::{debug, error, info, trace, warn};

use super::state::init_fcm_service_state;
use crate::{
    errors::{ChainTipError, Error},
    fcm::{
        context::{
            BlockExecutionOutcome, BlockValidationOutcome, ExecutionDeferral, FcmContext,
            FcmStorage,
        },
        input::FcmEvent,
        pending::{RETRY_BATCH_SIZE, STATUS_SCAN_SIZE},
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

#[derive(Clone, Debug, Default)]
pub(crate) struct FcmService<C: FcmContext>(PhantomData<C>);

#[derive(Clone, Debug, Serialize)]
pub struct FcmStatus {
    /// Number of blocks currently cached for dependency retries.
    pending_blocks: usize,
}

impl FcmStatus {
    /// Returns the number of blocks currently cached for dependency retries.
    pub fn pending_blocks(&self) -> usize {
        self.pending_blocks
    }
}

impl<C: FcmContext> Service for FcmService<C> {
    type Msg = FcmEvent;
    type State = FcmServiceState<C>;
    type Status = FcmStatus;

    fn get_status(state: &Self::State) -> Self::Status {
        FcmStatus {
            pending_blocks: state.pending_block_count(),
        }
    }
}

impl<C: FcmContext> AsyncService for FcmService<C> {
    async fn on_launch(state: &mut Self::State) -> anyhow::Result<()> {
        // The worker starts from the persisted canonical tip before FCM restores Valid blocks.
        // Reconcile it even when restoration leaves no blocks requiring execution replay.
        let restored_tip = state.cur_best_block();
        if let Err(error) = state.ctx().update_safe_tip(restored_tip).await {
            if !is_retryable_storage_error(&error) {
                return Err(error);
            }
            state.mark_fork_choice_pending();
            state.record_fork_choice_failure();
            warn!(%restored_tip, %error, "restored safe-tip update remains pending");
        }
        let startup_replay_candidates = state.take_startup_replay_candidates();
        if startup_replay_candidates.is_empty() {
            return Ok(());
        }

        let replay_candidate_count = startup_replay_candidates.len();
        for blkid in startup_replay_candidates {
            match state.ctx().get_ol_block(blkid).await {
                Ok(Some(block)) => state.discover_pending_block(&block),
                Ok(None) => warn!(%blkid, "startup replay block data is unavailable"),
                Err(error) => warn!(%blkid, %error, "failed to load startup replay block"),
            }
        }
        loop {
            let pending_before_retry = state.pending_block_count();
            retry_pending_blocks(state, true).await?;
            let pending_after_retry = state.pending_block_count();
            if pending_after_retry == 0 || pending_after_retry >= pending_before_retry {
                break;
            }
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
            FcmEvent::RetryTick => {}
            FcmEvent::Abort => return Ok(Response::ShouldExit),
        };
        retry_pending_blocks(fcm_state, !matches!(input, FcmEvent::RetryTick)).await?;
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

            if fcm_state.pending_needs_cleanup(block_bundle.header().compute_block_commitment()) {
                finish_rejection(fcm_state, &block_bundle).await?;
                return Ok(());
            }
            match fcm_state.ctx().get_block_status(*blkid).await {
                Ok(Some(BlockStatus::Invalid)) => {
                    finish_rejection(fcm_state, &block_bundle).await?;
                    return Ok(());
                }
                Err(error) if is_retryable_database_error(&error) => {
                    fcm_state.defer_block(&block_bundle, ExecutionDeferral::Storage);
                    warn!(%blkid, %error, "deferring block after status read failure");
                    return Ok(());
                }
                Err(error) => return Err(error.into()),
                _ => {}
            }

            let outcome = handle_new_block(fcm_state, &block_bundle).await?;

            let block = OLBlockCommitment::new(slot, *blkid);
            match outcome {
                BlockExecutionOutcome::Accepted => {
                    // handle_new_block persisted Valid before attaching the block.
                    fcm_state.remove_pending_block(block);
                    counter!("strata_fcm_blocks_accepted_total").increment(1);
                    retry_fork_choice(fcm_state)
                        .await
                        .context(ForkChoiceFailure)?;
                }
                BlockExecutionOutcome::Rejected => {
                    warn!(%blkid, "rejecting invalid block");
                    finish_rejection(fcm_state, &block_bundle).await?;
                    counter!("strata_fcm_blocks_rejected_total").increment(1);
                }
                BlockExecutionOutcome::Deferred(reason) => {
                    fcm_state.defer_block(&block_bundle, reason);
                    debug!(%blkid, ?reason, "deferring block execution");
                }
            }
        }
    }

    Ok(())
}

/// Recovers durable replayable blocks and retries a bounded, fair batch.
async fn retry_pending_blocks<C: FcmContext>(
    state: &mut FcmServiceState<C>,
    progress: bool,
) -> anyhow::Result<()> {
    let capacity = state.prepare_pending_refill(STATUS_SCAN_SIZE);
    if capacity > 0 {
        let previous_count = state.pending_block_count();
        // Reserve room for execution discovery when both scans find work. Rejection metadata
        // also covers finalized history, where unfinished cleanup can survive a restart.
        refill_pending_rejections(state, capacity.div_ceil(2)).await?;
        let added = state.pending_block_count() - previous_count;
        if capacity > added {
            refill_pending_blocks(state, capacity - added).await?;
        }
    }
    let mut attempted = Vec::with_capacity(RETRY_BATCH_SIZE);
    let mut progress = progress;
    while attempted.len() < RETRY_BATCH_SIZE {
        let candidates = state.select_pending_retry_batch(
            progress,
            RETRY_BATCH_SIZE - attempted.len(),
            &attempted,
        );
        if candidates.is_empty() {
            break;
        }
        for block in candidates {
            attempted.push(block);
            let was_attached = state.chain_tracker().is_seen_block(block.blkid());
            if let Err(err) = retry_pending_block(state, block).await {
                // Only temporary storage failures and absent bodies can recover here. Permanent
                // failures must reach the service monitor without changing a verdict.
                if err.is::<ForkChoiceFailure>()
                    || (!is_retryable_storage_error(&err)
                        && !matches!(err.downcast_ref::<Error>(), Some(Error::MissingOLBlock(_))))
                {
                    return Err(err);
                }
                state.record_pending_storage_failure(block);
                warn!(%block, %err, "failed to retry pending block");
            }
            // A newly attached parent makes its descendants eligible within this pass.
            progress |= !was_attached && state.chain_tracker().is_seen_block(block.blkid());
        }
    }
    retry_fork_choice(state).await?;
    Ok(())
}

/// Discovers unfinished rejection cleanup without loading completed or valid block bodies.
async fn refill_pending_rejections<C: FcmContext>(
    state: &mut FcmServiceState<C>,
    limit: usize,
) -> anyhow::Result<()> {
    match state
        .ctx()
        .scan_block_rejection_cleanup(state.rejection_scan_cursor(), limit)
        .await
    {
        Ok(rows) => {
            for &(id, pending) in &rows {
                if pending {
                    match state.ctx().get_ol_block(id).await {
                        Ok(Some(block)) => state.discover_pending_cleanup(&block),
                        Ok(None) => {}
                        Err(err) if is_retryable_database_error(&err) => {
                            // Revisit this row after wrapping, allowing other cleanup to proceed.
                            warn!(%id, %err, "failed to load rejected block for cleanup");
                        }
                        Err(err) => return Err(err.into()),
                    }
                }
            }
            state.record_rejection_status_page(&rows);
        }
        Err(err) if is_retryable_database_error(&err) => {
            warn!(%err, "failed to scan rejection cleanup metadata")
        }
        Err(err) => return Err(err.into()),
    }
    Ok(())
}

/// Scans only unfinalized metadata and loads bodies that can enter the cache.
async fn refill_pending_blocks<C: FcmContext>(
    state: &mut FcmServiceState<C>,
    limit: usize,
) -> anyhow::Result<()> {
    let Some(limit) = NonZeroUsize::new(limit) else {
        return Ok(());
    };
    let finalized_slot = state.chain_tracker().finalized_epoch().last_slot();
    let start = match state.pending_scan_cursor() {
        Some(cursor) if cursor.slot() > finalized_slot => StatusScanStart::AfterBlock(cursor),
        _ => StatusScanStart::AfterSlot(finalized_slot),
    };
    match state.ctx().scan_block_statuses(start, limit).await {
        Ok(rows) => {
            if rows.is_empty() {
                state.record_pending_status_page(&rows);
            }
            for (commitment, status) in rows {
                let id = *commitment.blkid();
                if status == BlockStatus::Invalid {
                    match state.ctx().is_block_rejection_complete(id).await {
                        Ok(true) => {
                            state.record_pending_status_page(&[(commitment, status)]);
                            continue;
                        }
                        Ok(false) => {}
                        Err(err) if is_retryable_database_error(&err) => {
                            // A later scan retries this metadata read without holding up other
                            // rows.
                            warn!(%id, %err, "failed to read rejection cleanup marker");
                            state.record_pending_status_page(&[(commitment, status)]);
                            continue;
                        }
                        Err(err) => return Err(err.into()),
                    }
                }
                let should_discover =
                    matches!(status, BlockStatus::Unchecked | BlockStatus::Invalid)
                        || (status == BlockStatus::Valid
                            && !state.chain_tracker().is_seen_block(&id));
                if should_discover {
                    match state.ctx().get_ol_block(id).await {
                        Ok(Some(block)) if status == BlockStatus::Invalid => {
                            state.discover_pending_cleanup(&block);
                        }
                        Ok(Some(block)) => state.discover_pending_block(&block),
                        Ok(None) => {}
                        Err(err) if is_retryable_database_error(&err) => {
                            if !state.record_pending_body_read_failure(commitment) {
                                warn!(%id, %err, "failed to load pending block");
                                break;
                            }
                            error!(%id, %err, "pending block body remains unreadable; continuing discovery and retrying it on the next scan cycle");
                        }
                        Err(err) => return Err(err.into()),
                    }
                }
                state.record_pending_status_page(&[(commitment, status)]);
            }
        }
        Err(err) if is_retryable_database_error(&err) => {
            warn!(%err, "failed to refill pending blocks")
        }
        Err(err) => return Err(err.into()),
    }
    Ok(())
}

async fn retry_pending_block<C: FcmContext>(
    state: &mut FcmServiceState<C>,
    block: OLBlockCommitment,
) -> anyhow::Result<()> {
    let id = *block.blkid();
    let status = match state.ctx().get_block_status(id).await {
        Ok(Some(status)) => status,
        Ok(None) => {
            state.remove_pending_block(block);
            return Ok(());
        }
        Err(error) => return Err(error.into()),
    };

    if status == BlockStatus::Invalid || state.pending_needs_cleanup(block) {
        let bundle = state
            .ctx()
            .get_ol_block(id)
            .await?
            .ok_or(Error::MissingOLBlock(id))?;
        finish_rejection(state, &bundle).await?;
        return Ok(());
    }

    counter!("strata_fcm_pending_retried_total").increment(1);
    match status {
        BlockStatus::Unchecked => process_fc_message(&ForkChoiceMessage::NewBlock(id), state).await,
        BlockStatus::Valid if !state.chain_tracker().is_seen_block(&id) => {
            retry_stored_valid_block(state, id).await
        }
        BlockStatus::Valid | BlockStatus::Invalid => {
            state.remove_pending_block(block);
            Ok(())
        }
    }
}

async fn retry_stored_valid_block<C: FcmContext>(
    state: &mut FcmServiceState<C>,
    id: OLBlockId,
) -> anyhow::Result<()> {
    let block = state
        .ctx()
        .get_ol_block(id)
        .await?
        .ok_or(Error::MissingOLBlock(id))?;
    let commitment = block.header().compute_block_commitment();

    match state.ctx().validate_block_inputs(commitment).await? {
        BlockValidationOutcome::Authenticated => {
            // Keep the durable verdict intact until replay produces an explicit rejection.
            process_fc_message(&ForkChoiceMessage::NewBlock(id), state).await
        }
        BlockValidationOutcome::Deferred(reason) => {
            state.defer_block(&block, reason);
            debug!(%id, ?reason, "deferring stored block authentication");
            Ok(())
        }
        BlockValidationOutcome::Rejected(error) => {
            warn!(%id, %error, "rejecting stored block with invalid inputs");
            finish_rejection(state, &block).await?;
            Ok(())
        }
    }
}

/// Retries idempotent invalid-block cleanup without reexecuting a rejected block.
async fn finish_rejection<C: FcmContext>(
    state: &mut FcmServiceState<C>,
    bundle: &OLBlockV1,
) -> anyhow::Result<()> {
    let block = bundle.header().compute_block_commitment();
    let result: anyhow::Result<()> = async {
        if state
            .ctx()
            .is_block_rejection_complete(*block.blkid())
            .await?
        {
            return Ok(());
        }
        set_block_status_and_clear_invalid_high_watermark(
            state,
            bundle,
            block,
            BlockStatus::Invalid,
        )
        .await?;
        state.ctx().mark_block_rejection_complete(block).await?;
        Ok(())
    }
    .await;
    match result {
        Ok(_) => state.remove_pending_block(block),
        Err(error) if is_retryable_storage_error(&error) => {
            state.discover_pending_cleanup(bundle);
            state.record_pending_storage_failure(block);
            warn!(%block, %error, "rejection cleanup remains pending after a local failure");
        }
        Err(error) => return Err(error),
    }
    Ok(())
}

async fn set_block_status_and_clear_invalid_high_watermark<C: FcmContext>(
    fcm_state: &FcmServiceState<C>,
    bundle: &OLBlockV1,
    block: OLBlockCommitment,
    status: BlockStatus,
) -> anyhow::Result<bool> {
    let updated = fcm_state
        .ctx()
        .set_block_status(*block.blkid(), status)
        .await?;

    if matches!(status, BlockStatus::Invalid) {
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
        Ok(Some(finalized_epoch)) => {
            publish_sync_status(fcm_state).await?;
            if finalized_epoch == observed_finalized_epoch {
                debug!(
                    ?finalized_epoch,
                    "FCM caught up to observed finalized epoch"
                );
            } else {
                debug!(
                    ?finalized_epoch,
                    ?observed_finalized_epoch,
                    "FCM finalized earlier recorded epoch; still behind observed finalized epoch"
                );
            }
        }
        Ok(None) => {
            // there were no epochs that could be finalized
            debug!(?observed_finalized_epoch, "no finalization progress");
        }
    };

    Ok(())
}

async fn publish_sync_status<C: FcmContext>(fcm_state: &FcmServiceState<C>) -> anyhow::Result<()> {
    let last_l1_blk = L1BlockCommitment::new(
        fcm_state.cur_ol_state().epoch_state().last_l1_height(),
        *fcm_state.cur_ol_state().epoch_state().last_l1_blkid(),
    );

    let cur_state = fcm_state.cur_ol_state();
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
    let status = OLSyncStatus::new(
        canonical_tip,
        tip_block_data.header().epoch(),
        tip_block_data.header().is_terminal(),
        prev_epoch,
        confirmed_epoch,
        finalized_epoch,
        last_l1_blk,
    );

    trace!(%canonical_tip, "publishing new ol_state");
    fcm_state.ctx().publish_sync_status(status);

    Ok(())
}

async fn handle_new_block<C: FcmContext>(
    fcm_state: &mut FcmServiceState<C>,
    bundle: &OLBlockV1,
) -> anyhow::Result<BlockExecutionOutcome> {
    let slot = bundle.header().slot();
    let blkid = &bundle.header().compute_blkid();
    info!(%blkid, %slot, "handling new block");

    // First, decide if the block seems correctly signed and we haven't
    // already marked it as invalid.
    if let Err(err) = check_ol_block_proposal_valid(blkid, bundle, fcm_state.sequencer_predicate())
    {
        warn!(%err, "rejecting block");
        return Ok(BlockExecutionOutcome::Rejected);
    }

    // A parent's state may exist before its status write or attachment completes. Executing its
    // child would advance indexing past the parent and prevent that parent's retry from succeeding.
    if !fcm_state.is_execution_ready(bundle) {
        return Ok(BlockExecutionOutcome::Deferred(
            ExecutionDeferral::Dependency,
        ));
    }

    // This stores the block output in the database, which lets us make queries
    // about it, at least until it gets reorged out by another block being
    // finalized.
    let bc = OLBlockCommitment::new(bundle.header().slot(), *blkid);
    let outcome = fcm_state.ctx().try_exec_block(bc).await?;

    if let BlockExecutionOutcome::Deferred(_) = outcome {
        return Ok(outcome);
    }
    if outcome == BlockExecutionOutcome::Rejected {
        return Ok(outcome);
    }
    if let Err(error) = fcm_state
        .ctx()
        .set_block_status(*blkid, BlockStatus::Valid)
        .await
    {
        if !is_retryable_database_error(&error) {
            return Err(error.into());
        }
        warn!(%blkid, %error, "deferring block after status write failure");
        return Ok(BlockExecutionOutcome::Deferred(ExecutionDeferral::Storage));
    }

    // Insert block into pending block tracker and figure out if we
    // should switch to it as a potential head.  This returns if we
    // created a new tip instead of advancing an existing tip.
    let new_tip = match fcm_state.chain_tracker_mut().attach_block(
        bundle.header().slot(),
        *blkid,
        *bundle.header().parent_blkid(),
    ) {
        Ok(new_tip) => new_tip,
        Err(ChainTipError::AttachMissingParent(_, parent_blkid)) => {
            debug!(%blkid, %parent_blkid, "deferring block whose parent is not attached");
            return Ok(BlockExecutionOutcome::Deferred(
                ExecutionDeferral::Dependency,
            ));
        }
        Err(error) => return Err(error.into()),
    };

    if new_tip {
        debug!(?blkid, "created new branching tip");
    }

    fcm_state.mark_fork_choice_pending();
    Ok(BlockExecutionOutcome::Accepted)
}

/// Identifies fatal fork-choice errors crossing the per-block storage retry boundary.
#[derive(Debug, ThisError)]
#[error("fork choice failed")]
struct ForkChoiceFailure;

/// Retries explicit contention and temporary I/O, including wrapped worker errors.
fn is_retryable_storage_error(error: &anyhow::Error) -> bool {
    let Some(database_error) = error
        .chain()
        .find_map(|source| source.downcast_ref::<DbError>())
    else {
        return false;
    };
    is_retryable_database_error(database_error)
}

fn is_retryable_database_error(error: &DbError) -> bool {
    error.is_retryable()
}

/// Retries fork choice even when every accepted block is already attached or evicted.
async fn retry_fork_choice<C: FcmContext>(state: &mut FcmServiceState<C>) -> anyhow::Result<()> {
    if !state.fork_choice_retry_due() {
        return Ok(());
    }
    match update_fork_choice(state).await {
        Ok(()) => state.complete_fork_choice(),
        Err(error) if is_retryable_storage_error(&error) => {
            state.record_fork_choice_failure();
            warn!(%error, "fork choice remains pending after a temporary database failure");
        }
        Err(error) => return Err(error),
    }
    Ok(())
}

async fn update_fork_choice<C: FcmContext>(state: &mut FcmServiceState<C>) -> anyhow::Result<()> {
    let cur_tip = *state.cur_best_block().blkid();
    let tips: Vec<OLBlockId> = state.chain_tracker().chain_tips_iter().copied().collect();
    let best_block = pick_best_block_async(&cur_tip, &tips, state.ctx()).await?;
    // TODO(STR-3050): make configurable
    let depth = 100;
    if let Some(update) = compute_tip_update(&cur_tip, &best_block, depth, state.chain_tracker())? {
        // The chosen tip can differ from the block whose arrival triggered fork choice.
        let bundle = state
            .ctx()
            .get_ol_block(best_block)
            .await?
            .ok_or(Error::MissingOLBlock(best_block))?;
        let old_tip = state.cur_best_block();
        let old_state = state.cur_ol_state();
        if let Err(error) = apply_tip_update(update, state, &bundle).await {
            // Reapply the entire path from its original pivot on retry. Keeping a partial
            // in-memory advance would skip failed safe-tip or canonical-prefix writes.
            state.set_tip_state(old_tip, old_state);
            return Err(error);
        }
        info!(%best_block, "new chain tip");
    } else {
        // Startup may already have selected this tip while the worker still has the old one.
        state.ctx().update_safe_tip(state.cur_best_block()).await?;
    }
    handle_epoch_finalization(state).await?;
    publish_sync_status(state).await?;
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
    block: &OLBlockV1,
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
    let mut best_header = storage
        .get_ol_header(best_tip)
        .await?
        .ok_or(Error::MissingOLBlock(best_tip))?;

    // The implementation of this will only switch to a new tip if it's a higher
    // height than our current tip.  We'll make this more sophisticated in the
    // future if we have a more sophisticated consensus protocol.
    for other_tip in tips {
        if other_tip == cur_tip {
            continue;
        }

        let other_header = storage
            .get_ol_header(*other_tip)
            .await?
            .ok_or(Error::MissingOLBlock(*other_tip))?;

        if other_header.slot() > best_header.slot() {
            best_tip = *other_tip;
            best_header = other_header;
        }
    }

    Ok(best_tip)
}

async fn apply_tip_update<C: FcmContext>(
    update: TipUpdate,
    fcm_state: &mut FcmServiceState<C>,
    bundle: &OLBlockV1,
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
        collections::{BTreeMap, BTreeSet, HashMap},
        sync::{Arc, Mutex},
        time::Duration,
    };

    use async_trait::async_trait;
    use strata_asm_common::AsmManifest;
    use strata_db_types::{ol_block::BlockStatus, DbError, DbResult};
    use strata_identifiers::{Epoch, Slot, WtxidsRoot};
    use strata_ol_chain_types_v1::{
        test_utils::{schnorr_predicate, test_schnorr_keypair},
        BlockFlagsV1, OLBlockBodyV1, OLBlockCredentialV1, OLBlockHeaderV1, OLBlockV1,
        OLTxSegmentV1, SignedOLBlockHeaderV1,
    };
    use strata_ol_state_support_types::MemoryStateBaseLayer;
    use strata_ol_state_types_v1::{OLStateV1, WriteBatch};
    use strata_ol_stf_v1::{
        test_utils::{execute_block, make_genesis_state},
        BlockComponents, BlockInfo, CompletedBlock, ExecError,
    };
    use strata_predicate::PredicateKey;
    use strata_primitives::{crypto::sign_schnorr_sig, l1::L1BlockId, Buf64, OLBlockId};
    use tokio::{
        pin,
        time::{advance, timeout},
    };

    use super::*;
    use crate::{
        fcm::{
            context::{
                BlockValidationOutcome, ChainController, CsmStatusReader, FcmStartupReconciler,
            },
            state::{reconcile_canonical_blocks_index, FcmInnerState},
            ExecutionDeferral,
        },
        ol_mmr_reconcile::{OLMmrReconcileResult, OLMmrReconcileTarget},
        tip_update::TipUpdate,
        unfinalized_tracker::{UnfinalizedBlockTracker, UnfinalizedOLBlockSource},
    };

    #[derive(Default)]
    struct StubFcmStorage {
        inner: Mutex<StubFcmStorageInner>,
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum CleanupFailure {
        Summary,
        WatermarkRead,
        Rollback,
        WatermarkClear,
        CompletionWrite,
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum StorageOperation {
        BodyRead,
        StatusRead,
        StatusWrite,
        StatusScan,
        RejectionScan,
        Cleanup,
    }

    #[derive(Default)]
    struct StubFcmStorageInner {
        cleanup_failure: Option<CleanupFailure>,
        storage_error: Option<(StorageOperation, DbError)>,
        body_read_failures: BTreeSet<OLBlockId>,
        status_read_failures: BTreeSet<OLBlockId>,
        status_writes_until_failure: Option<usize>,
        header_read_failures: BTreeSet<OLBlockId>,
        canonical_write_failures: usize,
        block_reads: Vec<OLBlockId>,
        status_reads: Vec<OLBlockId>,
        status_scans: usize,
        blocks: HashMap<OLBlockId, OLBlockV1>,
        headers: HashMap<OLBlockId, OLBlockHeaderV1>,
        statuses: HashMap<OLBlockId, BlockStatus>,
        completed_rejections: BTreeSet<OLBlockId>,
        blocks_by_slot: BTreeMap<Slot, Vec<OLBlockId>>,
        canonical_blocks: HashMap<Slot, OLBlockCommitment>,
        block_high_watermark: Option<OLBlockCommitment>,
        history_base: Option<EpochCommitment>,
        states: HashMap<OLBlockCommitment, Arc<OLStateV1>>,
        state_read_errors: HashMap<OLBlockCommitment, DbError>,
        canonical_epochs: HashMap<Epoch, EpochCommitment>,
        indexing_rollbacks: Vec<(Epoch, OLBlockCommitment)>,
        epoch_summary_deletes: Vec<EpochCommitment>,
    }

    impl StubFcmStorageInner {
        fn check_storage_error(&mut self, operation: StorageOperation) -> DbResult<()> {
            if self
                .storage_error
                .as_ref()
                .is_some_and(|(point, _)| *point == operation)
            {
                return Err(self.storage_error.take().expect("matching error exists").1);
            }
            Ok(())
        }

        fn fail_cleanup(&mut self, operation: CleanupFailure) -> DbResult<()> {
            if self.cleanup_failure == Some(operation) {
                self.cleanup_failure = None;
                return Err(DbError::Busy);
            }
            Ok(())
        }
    }

    impl StubFcmStorage {
        fn new() -> Self {
            Self::default()
        }

        fn put_ol_block(&self, block: OLBlockV1) -> OLBlockCommitment {
            self.put_block_parts(block, None, None)
        }

        fn put_ol_header(&self, header: OLBlockHeaderV1) -> OLBlockCommitment {
            let blkid = header.compute_blkid();
            let commitment = OLBlockCommitment::new(header.slot(), blkid);
            self.inner.lock().unwrap().headers.insert(blkid, header);
            commitment
        }

        fn put_executed_block(
            &self,
            block: OLBlockV1,
            state: OLStateV1,
            status: BlockStatus,
        ) -> OLBlockCommitment {
            self.put_block_parts(block, Some(state), Some(status))
        }

        fn put_block_parts(
            &self,
            block: OLBlockV1,
            state: Option<OLStateV1>,
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

        fn put_toplevel_ol_state(&self, block: OLBlockCommitment, state: OLStateV1) {
            let mut inner = self.inner.lock().unwrap();
            assert!(
                inner.blocks.contains_key(block.blkid())
                    || inner.headers.contains_key(block.blkid()),
                "cannot seed OL state without the corresponding OL block or header"
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
            let mut inner = self.inner.lock().unwrap();
            inner.block_high_watermark = Some(block);
            inner.completed_rejections.remove(block.blkid());
        }

        fn set_history_base(&self, history_base: EpochCommitment) {
            self.inner.lock().unwrap().history_base = Some(history_base);
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
        execution_outcomes: Mutex<HashMap<OLBlockId, BlockExecutionOutcome>>,
        execution_errors: Mutex<BTreeSet<OLBlockId>>,
        execution_statuses: Mutex<Vec<Option<BlockStatus>>>,
        validated_blocks: Mutex<Vec<OLBlockCommitment>>,
        validation_deferrals: Mutex<HashMap<OLBlockId, ExecutionDeferral>>,
        validation_failures: Mutex<BTreeSet<OLBlockId>>,
        validation_errors: Mutex<HashMap<OLBlockId, DbError>>,
        safe_tip_updates: Mutex<Vec<OLBlockCommitment>>,
        safe_tip_failures: Mutex<usize>,
        safe_tip_error: Mutex<Option<anyhow::Error>>,
        finalized_epochs: Mutex<Vec<EpochCommitment>>,
        published_statuses: Mutex<Vec<OLSyncStatus>>,
        startup_mmr_reconcile_targets: Mutex<Vec<OLMmrReconcileTarget>>,
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

        fn set_execution_outcome(&self, block: OLBlockId, outcome: BlockExecutionOutcome) {
            self.execution_outcomes
                .lock()
                .unwrap()
                .insert(block, outcome);
        }

        fn clear_execution_outcomes(&self) {
            self.execution_outcomes.lock().unwrap().clear();
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

        fn startup_mmr_reconcile_targets(&self) -> Vec<OLMmrReconcileTarget> {
            self.startup_mmr_reconcile_targets.lock().unwrap().clone()
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
            let mut inner = self.inner.lock().unwrap();
            inner.check_storage_error(StorageOperation::StatusRead)?;
            inner.status_reads.push(blkid);
            if inner.status_read_failures.remove(&blkid) {
                return Err(DbError::Busy);
            }
            Ok(inner.statuses.get(&blkid).copied())
        }

        async fn get_ol_block(&self, blkid: OLBlockId) -> DbResult<Option<OLBlockV1>> {
            let mut inner = self.inner.lock().unwrap();
            inner.check_storage_error(StorageOperation::BodyRead)?;
            inner.block_reads.push(blkid);
            if inner.body_read_failures.remove(&blkid) {
                return Err(DbError::Busy);
            }
            Ok(inner.blocks.get(&blkid).cloned())
        }

        async fn get_ol_header(&self, blkid: OLBlockId) -> DbResult<Option<OLBlockHeaderV1>> {
            let mut inner = self.inner.lock().unwrap();
            if inner.header_read_failures.remove(&blkid) {
                return Err(DbError::Busy);
            }
            Ok(inner
                .blocks
                .get(&blkid)
                .map(|block| block.header().clone())
                .or_else(|| inner.headers.get(&blkid).cloned()))
        }
    }

    #[async_trait]
    impl FcmStorage for StubFcmStorage {
        async fn scan_block_rejection_cleanup(
            &self,
            after: Option<OLBlockId>,
            limit: usize,
        ) -> DbResult<Vec<(OLBlockId, bool)>> {
            let mut inner = self.inner.lock().unwrap();
            inner.check_storage_error(StorageOperation::RejectionScan)?;
            let mut rows: Vec<_> = inner
                .statuses
                .iter()
                .filter(|(id, _)| after.is_none_or(|after| **id > after))
                .map(|(id, status)| {
                    (
                        *id,
                        *status == BlockStatus::Invalid && !inner.completed_rejections.contains(id),
                    )
                })
                .collect();
            rows.sort_by_key(|(id, _)| *id);
            rows.truncate(limit);
            Ok(rows)
        }

        async fn scan_block_statuses(
            &self,
            start: StatusScanStart,
            limit: NonZeroUsize,
        ) -> DbResult<Vec<(OLBlockCommitment, BlockStatus)>> {
            let mut inner = self.inner.lock().unwrap();
            inner.check_storage_error(StorageOperation::StatusScan)?;
            inner.status_scans += 1;
            let mut rows: Vec<_> = inner
                .blocks_by_slot
                .iter()
                .flat_map(|(slot, ids)| {
                    ids.iter().map(move |id| OLBlockCommitment::new(*slot, *id))
                })
                .filter(|block| match start {
                    StatusScanStart::AfterSlot(slot) => block.slot() > slot,
                    StatusScanStart::AfterBlock(after) => *block > after,
                })
                .filter_map(|block| {
                    inner
                        .statuses
                        .get(block.blkid())
                        .map(|status| (block, *status))
                })
                .collect();
            rows.sort_by_key(|(id, _)| *id);
            rows.truncate(limit.get());
            Ok(rows)
        }

        async fn set_block_status(&self, blkid: OLBlockId, status: BlockStatus) -> DbResult<bool> {
            let mut inner = self.inner.lock().unwrap();
            inner.check_storage_error(StorageOperation::StatusWrite)?;
            if let Some(remaining) = inner.status_writes_until_failure.as_mut() {
                if *remaining == 0 {
                    inner.status_writes_until_failure = None;
                    return Err(DbError::Busy);
                }
                *remaining -= 1;
            }
            let block_exists = inner.blocks.contains_key(&blkid);
            if block_exists {
                inner.statuses.insert(blkid, status);
                if status != BlockStatus::Invalid {
                    inner.completed_rejections.remove(&blkid);
                }
            }
            Ok(block_exists)
        }

        async fn is_block_rejection_complete(&self, blkid: OLBlockId) -> DbResult<bool> {
            self.inner
                .lock()
                .unwrap()
                .check_storage_error(StorageOperation::Cleanup)?;
            Ok(self
                .inner
                .lock()
                .unwrap()
                .completed_rejections
                .contains(&blkid))
        }

        async fn mark_block_rejection_complete(&self, block: OLBlockCommitment) -> DbResult<()> {
            let mut inner = self.inner.lock().unwrap();
            inner.fail_cleanup(CleanupFailure::CompletionWrite)?;
            let status = inner.statuses.get(block.blkid()).copied();
            if status != Some(BlockStatus::Invalid) {
                return Err(DbError::BlockRejectionStatusMismatch {
                    block_id: *block.blkid(),
                    status,
                });
            }
            if let Some(current) = inner
                .block_high_watermark
                .filter(|current| current.blkid() == block.blkid())
            {
                return Err(DbError::BlockRejectionHighWatermark(current));
            }
            inner.completed_rejections.insert(*block.blkid());
            Ok(())
        }

        async fn clear_block_high_watermark(&self, expected: OLBlockCommitment) -> DbResult<bool> {
            let mut inner = self.inner.lock().unwrap();
            inner.fail_cleanup(CleanupFailure::WatermarkClear)?;
            if inner.block_high_watermark != Some(expected) {
                return Ok(false);
            }

            inner.block_high_watermark = None;
            Ok(true)
        }

        async fn get_block_high_watermark(&self) -> DbResult<Option<OLBlockCommitment>> {
            let mut inner = self.inner.lock().unwrap();
            inner.fail_cleanup(CleanupFailure::WatermarkRead)?;
            Ok(inner.block_high_watermark)
        }

        async fn get_history_base(&self) -> DbResult<Option<EpochCommitment>> {
            Ok(self.inner.lock().unwrap().history_base)
        }

        async fn rollback_block_state_indexing(
            &self,
            epoch: Epoch,
            cutoff: OLBlockCommitment,
        ) -> DbResult<()> {
            let mut inner = self.inner.lock().unwrap();
            inner.fail_cleanup(CleanupFailure::Rollback)?;
            inner.indexing_rollbacks.push((epoch, cutoff));
            Ok(())
        }

        async fn del_epoch_summary(&self, epoch: EpochCommitment) -> DbResult<bool> {
            let mut inner = self.inner.lock().unwrap();
            inner.fail_cleanup(CleanupFailure::Summary)?;
            inner.epoch_summary_deletes.push(epoch);
            Ok(false)
        }

        async fn get_toplevel_ol_state(
            &self,
            commitment: OLBlockCommitment,
        ) -> DbResult<Option<Arc<OLStateV1>>> {
            let mut inner = self.inner.lock().unwrap();
            if let Some(error) = inner.state_read_errors.remove(&commitment) {
                return Err(error);
            }
            Ok(inner.states.get(&commitment).cloned())
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
            if inner.canonical_write_failures > 0 {
                inner.canonical_write_failures -= 1;
                return Err(DbError::Busy);
            }
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
        async fn try_exec_block(
            &self,
            block: OLBlockCommitment,
        ) -> anyhow::Result<BlockExecutionOutcome> {
            self.executed_blocks.lock().unwrap().push(block);
            let status = self.storage.get_block_status(*block.blkid()).await?;
            self.execution_statuses.lock().unwrap().push(status);
            anyhow::ensure!(
                !self
                    .execution_errors
                    .lock()
                    .unwrap()
                    .contains(block.blkid()),
                "injected unclassified worker failure"
            );
            Ok(self
                .execution_outcomes
                .lock()
                .unwrap()
                .get(block.blkid())
                .copied()
                .unwrap_or(BlockExecutionOutcome::Accepted))
        }

        async fn validate_block_inputs(
            &self,
            block: OLBlockCommitment,
        ) -> anyhow::Result<BlockValidationOutcome> {
            self.validated_blocks.lock().unwrap().push(block);
            if let Some(error) = self.validation_errors.lock().unwrap().get(block.blkid()) {
                return Err(error.clone().into());
            }
            if let Some(reason) = self
                .validation_deferrals
                .lock()
                .unwrap()
                .get(block.blkid())
                .copied()
            {
                return Ok(BlockValidationOutcome::Deferred(reason));
            }
            if self
                .validation_failures
                .lock()
                .unwrap()
                .contains(block.blkid())
            {
                return Ok(BlockValidationOutcome::Rejected(
                    ExecError::AsmManifestHeightOverflow,
                ));
            }
            Ok(BlockValidationOutcome::Authenticated)
        }

        async fn update_safe_tip(&self, safe_tip: OLBlockCommitment) -> anyhow::Result<()> {
            if let Some(error) = self.safe_tip_error.lock().unwrap().take() {
                return Err(error);
            }
            let mut failures = self.safe_tip_failures.lock().unwrap();
            if *failures > 0 {
                *failures -= 1;
                return Err(DbError::Busy.into());
            }
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

        async fn get_ol_block(&self, blkid: OLBlockId) -> DbResult<Option<OLBlockV1>> {
            self.storage.get_ol_block(blkid).await
        }

        async fn get_ol_header(&self, blkid: OLBlockId) -> DbResult<Option<OLBlockHeaderV1>> {
            self.storage.get_ol_header(blkid).await
        }
    }

    #[async_trait]
    impl FcmStorage for StubFcmContext {
        async fn scan_block_rejection_cleanup(
            &self,
            after: Option<OLBlockId>,
            limit: usize,
        ) -> DbResult<Vec<(OLBlockId, bool)>> {
            self.storage
                .scan_block_rejection_cleanup(after, limit)
                .await
        }

        async fn scan_block_statuses(
            &self,
            start: StatusScanStart,
            limit: NonZeroUsize,
        ) -> DbResult<Vec<(OLBlockCommitment, BlockStatus)>> {
            self.storage.scan_block_statuses(start, limit).await
        }

        async fn set_block_status(&self, blkid: OLBlockId, status: BlockStatus) -> DbResult<bool> {
            self.storage.set_block_status(blkid, status).await
        }

        async fn is_block_rejection_complete(&self, blkid: OLBlockId) -> DbResult<bool> {
            self.storage.is_block_rejection_complete(blkid).await
        }

        async fn mark_block_rejection_complete(&self, block: OLBlockCommitment) -> DbResult<()> {
            self.storage.mark_block_rejection_complete(block).await
        }

        async fn clear_block_high_watermark(&self, expected: OLBlockCommitment) -> DbResult<bool> {
            self.storage.clear_block_high_watermark(expected).await
        }

        async fn get_block_high_watermark(&self) -> DbResult<Option<OLBlockCommitment>> {
            self.storage.get_block_high_watermark().await
        }

        async fn get_history_base(&self) -> DbResult<Option<EpochCommitment>> {
            self.storage.get_history_base().await
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
        ) -> DbResult<Option<Arc<OLStateV1>>> {
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

    #[async_trait]
    impl FcmStartupReconciler for StubFcmContext {
        async fn reconcile_ol_mmr_index(
            &self,
            target: OLMmrReconcileTarget,
        ) -> OLMmrReconcileResult<()> {
            self.startup_mmr_reconcile_targets
                .lock()
                .unwrap()
                .push(target);
            Ok(())
        }
    }

    impl FcmContext for StubFcmContext {
        fn publish_sync_status(&self, status: OLSyncStatus) {
            self.published_statuses.lock().unwrap().push(status);
        }
    }

    #[derive(Clone)]
    struct ExecutedBlock {
        block: OLBlockV1,
        state: OLStateV1,
    }

    impl ExecutedBlock {
        fn new(completed: CompletedBlock, state: &MemoryStateBaseLayer) -> Self {
            let signed_header =
                SignedOLBlockHeaderV1::new(completed.header().clone(), Buf64::zero());
            Self {
                block: OLBlockV1::new(signed_header, completed.body().clone()),
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
        parent: &OLBlockV1,
        timestamp: u64,
        slot: u64,
    ) -> ExecutedBlock {
        execute_test_block_in_epoch(state, parent, timestamp, slot, 1)
    }

    fn execute_test_block_in_epoch(
        state: &mut MemoryStateBaseLayer,
        parent: &OLBlockV1,
        timestamp: u64,
        slot: u64,
        epoch: Epoch,
    ) -> ExecutedBlock {
        execute_test_block_with_components(
            state,
            parent,
            timestamp,
            slot,
            epoch,
            BlockComponents::new_empty(),
        )
    }

    fn execute_terminal_test_block_in_epoch(
        state: &mut MemoryStateBaseLayer,
        parent: &OLBlockV1,
        timestamp: u64,
        slot: u64,
        epoch: Epoch,
    ) -> ExecutedBlock {
        execute_test_block_with_components(
            state,
            parent,
            timestamp,
            slot,
            epoch,
            BlockComponents::new_empty().as_terminal(),
        )
    }

    fn execute_test_block_with_components(
        state: &mut MemoryStateBaseLayer,
        parent: &OLBlockV1,
        timestamp: u64,
        slot: u64,
        epoch: Epoch,
        components: BlockComponents,
    ) -> ExecutedBlock {
        let completed = execute_block(
            state,
            &BlockInfo::new(timestamp, slot, epoch),
            Some(parent.header()),
            components,
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

    #[tokio::test(start_paused = true)]
    async fn retry_pass_attaches_a_recovered_chain_up_to_the_batch_limit() -> anyhow::Result<()> {
        let (genesis, mut chain_state) = execute_test_genesis();
        let mut blocks: Vec<ExecutedBlock> = Vec::new();
        for slot in 1..=(RETRY_BATCH_SIZE + 2) as u64 {
            let parent = blocks.last().unwrap_or(&genesis);
            blocks.push(execute_test_block(
                &mut chain_state,
                &parent.block,
                1_000 + slot * 100,
                slot,
            ));
        }
        let fixture = FcmTestFixture::new(&genesis, &[]);
        let mut state = fixture.fcm_state_at(empty_tracker(&genesis), &genesis);
        for block in blocks.iter().rev() {
            seed_executed_block(fixture.ctx.storage(), block, BlockStatus::Unchecked);
            state.defer_block(&block.block, ExecutionDeferral::Dependency);
        }
        advance(Duration::from_secs(1)).await;

        retry_pending_blocks(&mut state, false).await?;

        let expected: Vec<_> = blocks.iter().map(ExecutedBlock::commitment).collect();
        assert_eq!(fixture.ctx.executed_blocks(), expected[..RETRY_BATCH_SIZE]);
        assert_eq!(state.cur_best_block(), expected[RETRY_BATCH_SIZE - 1]);
        assert_eq!(state.pending_block_count(), 2);

        retry_pending_blocks(&mut state, false).await?;
        assert_eq!(fixture.ctx.executed_blocks(), expected);
        assert_eq!(state.pending_block_count(), 0);
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn retry_pass_attempts_a_still_deferred_block_only_once() -> anyhow::Result<()> {
        let chain = LinearChain::new();
        let fixture = chain.fixture_without_x4();
        let mut state = fixture.fcm_state_at(empty_tracker(&chain.genesis), &chain.genesis);
        fixture.ctx.set_execution_outcome(
            chain.x1.blkid(),
            BlockExecutionOutcome::Deferred(ExecutionDeferral::Dependency),
        );
        state.defer_block(&chain.x1.block, ExecutionDeferral::Dependency);

        retry_pending_blocks(&mut state, true).await?;

        assert_eq!(fixture.ctx.executed_blocks(), vec![chain.x1.commitment()]);
        assert_eq!(state.cur_best_block(), chain.genesis.commitment());
        Ok(())
    }

    #[tokio::test]
    async fn get_block_slot_resolves_header_only_reorg_pivot() {
        let chain = LinearChain::new();
        let fixture = chain.fixture_without_x4();
        let finalized_epoch =
            EpochCommitment::new(1, chain.x1.commitment().slot(), chain.x1.blkid());
        let tracker = UnfinalizedBlockTracker::new_empty(finalized_epoch);
        let fcm_state = fixture.fcm_state_at(tracker, &chain.x1);

        // A history-base anchor exists only as a terminal-header record; a
        // reorg pivoting to it must still resolve its slot.
        let anchor = make_storage_block(7, OLBlockId::from(Buf32::zero()));
        let anchor_id = anchor.header().compute_blkid();
        fixture.ctx.storage().put_ol_header(anchor.header().clone());
        assert_eq!(
            fixture.ctx.storage().get_ol_block(anchor_id).await.unwrap(),
            None,
            "anchor must be header-only for this test"
        );

        assert_eq!(
            fcm_state
                .get_block_slot(anchor_id)
                .await
                .expect("header-only slot lookup"),
            7
        );
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
        seed_executed_block(ctx.storage(), &chain.genesis, BlockStatus::Valid);
        seed_executed_block(ctx.storage(), &chain.x1, BlockStatus::Valid);
        seed_executed_block(ctx.storage(), &chain.x2, BlockStatus::Valid);
        ctx.storage().put_canonical_epoch_commitment(pending_epoch);
        let tracker = tracker_with_blocks(&chain.genesis, &[&chain.x1, &chain.x2]);
        let mut finalizable_state = chain.x2.state.clone();
        let mut epoch_update = WriteBatch::default();
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
        let statuses = ctx.published_statuses();
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].finalized_epoch(), pending_epoch);

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
        assert_eq!(status.tip(), fork.b3.commitment());
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
    async fn startup_authenticates_valid_blocks_before_any_reconciliation() -> anyhow::Result<()> {
        let chain = LinearChain::new();
        let fixture = FcmTestFixture::new(
            &chain.genesis,
            &[&chain.x1, &chain.x2, &chain.x3, &chain.x4],
        );
        let ctx = fixture.ctx;
        ctx.storage()
            .replace_canonical_suffix_from(1, vec![chain.x1.blkid(), chain.x2.blkid()])
            .await?;
        ctx.validation_failures
            .lock()
            .unwrap()
            .insert(chain.x2.blkid());

        let result = init_fcm_service_state(PredicateKey::always_accept(), ctx.clone()).await;
        assert!(result
            .err()
            .unwrap()
            .to_string()
            .contains("cannot authenticate restored"));
        assert_eq!(
            ctx.storage().get_canonical_block_at(2).await?,
            Some(chain.x2.commitment())
        );
        assert_eq!(
            ctx.storage().get_block_status(chain.x2.blkid()).await?,
            Some(BlockStatus::Valid)
        );
        assert!(ctx.executed_blocks().is_empty());
        assert!(ctx.safe_tip_updates().is_empty());
        assert!(ctx.published_statuses().is_empty());
        assert!(ctx.startup_mmr_reconcile_targets().is_empty());
        assert!(ctx.storage().indexing_rollbacks().is_empty());

        ctx.validation_failures.lock().unwrap().clear();
        ctx.validated_blocks.lock().unwrap().clear();
        let state = init_fcm_service_state(PredicateKey::always_accept(), ctx.clone()).await?;
        assert_eq!(state.cur_best_block(), chain.x4.commitment());
        assert_eq!(
            *ctx.validated_blocks.lock().unwrap(),
            vec![
                chain.x1.commitment(),
                chain.x2.commitment(),
                chain.x3.commitment(),
                chain.x4.commitment()
            ]
        );
        assert!(ctx.executed_blocks().is_empty());
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn startup_propagates_authentication_database_errors_without_retrying(
    ) -> anyhow::Result<()> {
        for error in [
            DbError::CodecError("corrupt parent state".to_owned()),
            DbError::WorkerFailedStrangely("database worker exited".to_owned()),
        ] {
            let chain = LinearChain::new();
            let blocks = [&chain.x1, &chain.x2, &chain.x3, &chain.x4];
            let fixture = FcmTestFixture::new(&chain.genesis, &blocks);
            let ctx = fixture.ctx;
            ctx.storage()
                .replace_canonical_suffix_from(
                    1,
                    blocks.iter().map(|block| block.blkid()).collect(),
                )
                .await?;
            let expected = error.to_string();
            ctx.validation_errors
                .lock()
                .unwrap()
                .insert(chain.x2.blkid(), error);

            let result = timeout(
                Duration::from_secs(1),
                init_fcm_service_state(PredicateKey::always_accept(), ctx.clone()),
            )
            .await
            .expect("permanent authentication failures must not retry");
            let failure = match result {
                Err(failure) => failure,
                Ok(_) => panic!("permanent authentication failures must abort startup"),
            };
            assert_eq!(
                failure.downcast_ref::<DbError>().unwrap().to_string(),
                expected
            );
            assert_eq!(
                *ctx.validated_blocks.lock().unwrap(),
                vec![chain.x1.commitment(), chain.x2.commitment()]
            );
            for block in blocks {
                assert_eq!(
                    ctx.storage()
                        .get_canonical_block_at(block.commitment().slot())
                        .await?,
                    Some(block.commitment())
                );
                assert_eq!(
                    ctx.get_block_status(block.blkid()).await?,
                    Some(BlockStatus::Valid)
                );
            }
            assert!(ctx.executed_blocks().is_empty());
            assert!(ctx.safe_tip_updates().is_empty());
            assert!(ctx.published_statuses().is_empty());
            assert!(ctx.startup_mmr_reconcile_targets().is_empty());
            assert!(ctx.storage().indexing_rollbacks().is_empty());
        }
        Ok(())
    }

    #[tokio::test]
    async fn startup_preserves_indexes_when_valid_block_data_is_missing() -> anyhow::Result<()> {
        let chain = LinearChain::new();
        let blocks = [&chain.x1, &chain.x2, &chain.x3, &chain.x4];
        let fixture = FcmTestFixture::new(&chain.genesis, &blocks);
        let ctx = fixture.ctx;
        ctx.storage()
            .replace_canonical_suffix_from(1, blocks.iter().map(|block| block.blkid()).collect())
            .await?;
        ctx.storage()
            .inner
            .lock()
            .unwrap()
            .blocks
            .remove(&chain.x2.blkid());

        let error = match init_fcm_service_state(PredicateKey::always_accept(), ctx.clone()).await {
            Err(error) => error,
            Ok(_) => panic!("missing accepted block data must prevent startup reconciliation"),
        };
        assert!(error
            .to_string()
            .contains("missing stored valid block data"));
        for block in blocks {
            assert_eq!(
                ctx.storage()
                    .get_canonical_block_at(block.commitment().slot())
                    .await?,
                Some(block.commitment())
            );
            assert_eq!(
                ctx.storage().get_block_status(block.blkid()).await?,
                Some(BlockStatus::Valid)
            );
        }
        assert!(ctx.executed_blocks().is_empty());
        assert!(ctx.safe_tip_updates().is_empty());
        assert!(ctx.published_statuses().is_empty());
        assert!(ctx.startup_mmr_reconcile_targets().is_empty());
        assert!(ctx.storage().indexing_rollbacks().is_empty());
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn startup_waits_for_authentication_before_reconciling_indexes() -> anyhow::Result<()> {
        for reason in [ExecutionDeferral::Dependency, ExecutionDeferral::Storage] {
            let chain = LinearChain::new();
            let fixture = FcmTestFixture::new(
                &chain.genesis,
                &[&chain.x1, &chain.x2, &chain.x3, &chain.x4],
            );
            let ctx = fixture.ctx;
            let blocks = [&chain.x1, &chain.x2, &chain.x3, &chain.x4];
            ctx.storage()
                .replace_canonical_suffix_from(
                    1,
                    blocks.iter().map(|block| block.blkid()).collect(),
                )
                .await?;
            ctx.validation_deferrals
                .lock()
                .unwrap()
                .insert(chain.x2.blkid(), reason);

            let startup = init_fcm_service_state(PredicateKey::always_accept(), ctx.clone());
            pin!(startup);
            assert!(timeout(Duration::from_secs(2), startup.as_mut())
                .await
                .is_err());
            for block in blocks {
                assert_eq!(
                    ctx.storage()
                        .get_canonical_block_at(block.commitment().slot())
                        .await?,
                    Some(block.commitment())
                );
                assert_eq!(
                    ctx.get_block_status(block.blkid()).await?,
                    Some(BlockStatus::Valid)
                );
            }
            assert!(ctx.startup_mmr_reconcile_targets().is_empty());
            assert!(ctx.storage().indexing_rollbacks().is_empty());
            assert!(ctx.executed_blocks().is_empty());
            assert!(ctx.safe_tip_updates().is_empty());
            assert!(ctx.published_statuses().is_empty());

            ctx.validation_deferrals.lock().unwrap().clear();
            let mut state = startup.await?;
            assert_eq!(state.cur_best_block(), chain.x4.commitment());
            assert!(state.take_startup_replay_candidates().is_empty());
            assert_eq!(ctx.startup_mmr_reconcile_targets().len(), 1);
            assert_eq!(
                ctx.startup_mmr_reconcile_targets()[0].block,
                chain.x4.commitment()
            );
            assert!(ctx.executed_blocks().is_empty());
        }
        Ok(())
    }

    #[tokio::test]
    async fn startup_preserves_canonical_tip_on_equal_slot_forks() -> anyhow::Result<()> {
        let fork = TestFork::new();
        let fixture = fork.fixture();
        let (canonical_1, canonical_2, other_1, other_2) = if fork.a2.blkid() > fork.b2.blkid() {
            (&fork.a1, &fork.a2, &fork.b1, &fork.b2)
        } else {
            (&fork.b1, &fork.b2, &fork.a1, &fork.a2)
        };
        assert!(canonical_2.blkid() > other_2.blkid());

        fixture
            .ctx
            .storage()
            .replace_canonical_suffix_from(1, vec![canonical_1.blkid(), canonical_2.blkid()])
            .await?;

        let fcm_state = init_fcm_service_state(PredicateKey::always_accept(), fixture.ctx).await?;

        assert_eq!(fcm_state.cur_best_block(), canonical_2.commitment());
        let reconcile_targets = fcm_state.ctx().startup_mmr_reconcile_targets();
        assert_eq!(reconcile_targets.len(), 1);
        assert_eq!(
            reconcile_targets[0].rejected_indexing_blocks,
            BTreeSet::from([other_1.commitment(), other_2.commitment()])
        );

        Ok(())
    }

    #[tokio::test]
    async fn test_startup_reconciles_mmr_to_repaired_tip() -> anyhow::Result<()> {
        let chain = LinearChain::new();
        let mut state = MemoryStateBaseLayer::new(chain.x4.state.clone());
        let terminal =
            execute_terminal_test_block_in_epoch(&mut state, &chain.x4.block, 3_500, 5, 1);
        let fixture = FcmTestFixture::new(
            &chain.genesis,
            &[&chain.x1, &chain.x2, &chain.x3, &chain.x4, &terminal],
        );

        assert_eq!(terminal.block.header().epoch(), 1);
        assert_eq!(terminal.state.epoch_state().cur_epoch(), 2);

        let fcm_state =
            init_fcm_service_state(PredicateKey::always_accept(), fixture.ctx.clone()).await?;

        assert_eq!(fcm_state.cur_best_block(), terminal.commitment());
        let reconcile_targets = fixture.ctx.startup_mmr_reconcile_targets();
        assert_eq!(reconcile_targets.len(), 1);
        let reconcile_target = &reconcile_targets[0];
        assert_eq!(reconcile_target.block, terminal.commitment());
        assert_eq!(reconcile_target.epoch, terminal.block.header().epoch());
        assert_eq!(
            reconcile_target.state.global_state().get_cur_slot(),
            terminal.state.global_state().get_cur_slot()
        );
        assert!(reconcile_target.rejected_indexing_blocks.is_empty());

        Ok(())
    }

    #[tokio::test]
    async fn test_startup_mmr_reconcile_stops_at_promoted_history_base() -> anyhow::Result<()> {
        let chain = LinearChain::new();
        let mut state = MemoryStateBaseLayer::new(chain.x4.state.clone());
        let terminal =
            execute_terminal_test_block_in_epoch(&mut state, &chain.x4.block, 3_500, 5, 1);
        let history_base = EpochCommitment::new(
            terminal.block.header().epoch(),
            terminal.commitment().slot(),
            terminal.blkid(),
        );
        let ctx = Arc::new(
            StubFcmContext::new()
                .with_last_finalized_epoch(Some(history_base))
                .with_last_confirmed_epoch(Some(history_base)),
        );

        // A promoted checkpoint-sync datadir has the anchor header/state but may
        // not retain the full intra-epoch parent chain below it.
        ctx.storage().seed_canonical_genesis(chain.genesis.blkid());
        ctx.storage().put_ol_header(terminal.block.header().clone());
        ctx.storage()
            .put_toplevel_ol_state(terminal.commitment(), terminal.state.clone());
        ctx.storage().set_history_base(history_base);
        ctx.storage()
            .replace_canonical_suffix_from(terminal.commitment().slot(), vec![terminal.blkid()])
            .await?;

        assert_eq!(ctx.storage().get_ol_block(terminal.blkid()).await?, None);
        assert_eq!(ctx.storage().get_ol_header(chain.x4.blkid()).await?, None);

        let fcm_state = init_fcm_service_state(PredicateKey::always_accept(), ctx.clone()).await?;

        assert_eq!(fcm_state.cur_best_block(), terminal.commitment());
        let reconcile_targets = ctx.startup_mmr_reconcile_targets();
        assert_eq!(reconcile_targets.len(), 1);
        assert_eq!(reconcile_targets[0].block, terminal.commitment());
        assert_eq!(reconcile_targets[0].epoch, terminal.block.header().epoch());
        assert!(reconcile_targets[0].rejected_indexing_blocks.is_empty());

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
        assert_eq!(status.tip(), chain.x4.commitment());
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

    fn make_block(slot: u64, signature: Option<Buf64>) -> OLBlockV1 {
        let body = OLBlockBodyV1::new_common(OLTxSegmentV1::new(vec![]).expect("empty tx segment"));
        let header = OLBlockHeaderV1::new(
            1_000 + slot,
            BlockFlagsV1::zero(),
            slot,
            0,
            OLBlockId::from(Buf32::zero()),
            body.compute_hash_commitment(),
            Buf32::zero(),
            Buf32::zero(),
        );
        let signed_header = match signature {
            Some(signature) => SignedOLBlockHeaderV1::new(header, signature),
            None => SignedOLBlockHeaderV1 {
                header,
                credential: OLBlockCredentialV1 {
                    schnorr_sig: None::<Buf64>.into(),
                },
            },
        };

        OLBlockV1::new(signed_header, body)
    }

    fn make_storage_block(slot: Slot, parent: OLBlockId) -> OLBlockV1 {
        let body = OLBlockBodyV1::new_common(OLTxSegmentV1::new(vec![]).expect("empty tx segment"));
        let header = OLBlockHeaderV1::new(
            1_000 + slot,
            BlockFlagsV1::zero(),
            slot,
            0,
            parent,
            body.compute_hash_commitment(),
            Buf32::zero(),
            Buf32::zero(),
        );
        OLBlockV1::new(SignedOLBlockHeaderV1::new(header, Buf64::zero()), body)
    }

    fn make_terminal_storage_block(slot: Slot, parent: OLBlockId) -> OLBlockV1 {
        let body = OLBlockBodyV1::new_common(OLTxSegmentV1::new(vec![]).expect("empty tx segment"));
        let mut flags = BlockFlagsV1::zero();
        flags.set_is_terminal(true);
        let header = OLBlockHeaderV1::new(
            1_000 + slot,
            flags,
            slot,
            0,
            parent,
            body.compute_hash_commitment(),
            Buf32::zero(),
            Buf32::zero(),
        );
        OLBlockV1::new(SignedOLBlockHeaderV1::new(header, Buf64::zero()), body)
    }

    fn sign_block(block: &OLBlockV1, signing_key: &Buf32) -> Buf64 {
        let msg: Buf32 = block.header().compute_blkid().into();
        sign_schnorr_sig(&msg, signing_key)
    }

    #[tokio::test(start_paused = true)]
    async fn startup_synchronizes_valid_tip_without_execution_replay() {
        for fail_first_update in [false, true] {
            let (genesis, mut state) = execute_test_genesis();
            let block = execute_test_block(&mut state, &genesis.block, 1_001, 1);
            let fixture = FcmTestFixture::new(&genesis, &[&block]);
            // Only genesis is canonical on disk, as after an interrupted tip update.
            assert_eq!(
                fixture
                    .ctx
                    .storage()
                    .get_canonical_block_at(1)
                    .await
                    .unwrap(),
                None
            );
            let mut fcm =
                init_fcm_service_state(PredicateKey::always_accept(), fixture.ctx.clone())
                    .await
                    .unwrap();
            assert_eq!(fcm.cur_best_block(), block.commitment());
            assert!(fcm.take_startup_replay_candidates().is_empty());
            *fixture.ctx.safe_tip_failures.lock().unwrap() = usize::from(fail_first_update);

            <FcmService<StubFcmContext> as AsyncService>::on_launch(&mut fcm)
                .await
                .unwrap();
            if fail_first_update {
                assert!(fixture.ctx.safe_tip_updates().is_empty());
                <FcmService<StubFcmContext> as AsyncService>::process_input(
                    &mut fcm,
                    FcmEvent::NewStateUpdate,
                )
                .await
                .unwrap();
                assert!(fixture.ctx.safe_tip_updates().is_empty());
                advance(Duration::from_secs(1)).await;
                <FcmService<StubFcmContext> as AsyncService>::process_input(
                    &mut fcm,
                    FcmEvent::RetryTick,
                )
                .await
                .unwrap();
            }

            assert_eq!(fixture.ctx.safe_tip_updates(), vec![block.commitment()]);
            assert_eq!(
                fixture
                    .ctx
                    .storage()
                    .get_canonical_block_at(1)
                    .await
                    .unwrap(),
                Some(block.commitment())
            );
            assert_eq!(
                fixture.ctx.get_block_status(block.blkid()).await.unwrap(),
                Some(BlockStatus::Valid)
            );
            assert_eq!(fcm.pending_block_count(), 0);
            assert!(fixture.ctx.executed_blocks().is_empty());
            assert!(!fcm.fork_choice_retry_due());
        }
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
        assert_eq!(
            ctx.safe_tip_updates(),
            vec![genesis_commitment, commitment1, commitment2]
        );
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
    async fn picker_compares_header_only_current_tip() {
        let storage = StubFcmStorage::new();
        let current = make_storage_block(2, OLBlockId::from(Buf32::zero()));
        let current_id = current.header().compute_blkid();
        storage.put_ol_header(current.header().clone());

        let candidate = make_storage_block(3, current_id);
        let candidate_id = candidate.header().compute_blkid();
        storage.put_ol_block(candidate);

        assert_eq!(storage.get_ol_block(current_id).await.unwrap(), None);
        assert_eq!(
            pick_best_block_async(&current_id, &[current_id, candidate_id], &storage)
                .await
                .expect("pick best block"),
            candidate_id
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
        assert_eq!(statuses[0].tip(), block_commitment);
        assert_eq!(statuses[0].recently_complete_epoch(), genesis_epoch);
        assert_eq!(statuses[0].confirmed_epoch(), genesis_epoch);
        assert_eq!(statuses[0].finalized_epoch(), genesis_epoch);
    }

    #[tokio::test]
    async fn pending_parent_and_child_resume_in_order() {
        let (genesis, mut state) = execute_test_genesis();
        let parent = execute_test_block(&mut state, &genesis.block, 1_001, 1);
        let child = execute_test_block(&mut state, &parent.block, 1_002, 2);
        let fixture = FcmTestFixture::new(&genesis, &[]);
        for block in [&parent, &child] {
            seed_executed_block(fixture.ctx.storage(), block, BlockStatus::Unchecked);
            fixture.ctx.set_execution_outcome(
                block.blkid(),
                BlockExecutionOutcome::Deferred(ExecutionDeferral::Dependency),
            );
        }
        let mut fcm = fixture.fcm_state_at(empty_tracker(&genesis), &genesis);
        for block in [&child, &parent] {
            process_fc_message(&ForkChoiceMessage::NewBlock(block.blkid()), &mut fcm)
                .await
                .unwrap();
        }
        let previous_attempt_count = fixture.ctx.executed_blocks().len();
        fixture.ctx.clear_execution_outcomes();
        retry_pending_blocks(&mut fcm, true).await.unwrap();
        retry_pending_blocks(&mut fcm, true).await.unwrap();
        assert_eq!(
            &fixture.ctx.executed_blocks()[previous_attempt_count..],
            &[parent.commitment(), child.commitment()]
        );
        assert_eq!(fcm.cur_best_block(), child.commitment());
        assert_eq!(fcm.pending_block_count(), 0);
        for block in [&parent, &child] {
            assert_eq!(
                fixture.ctx.get_block_status(block.blkid()).await.unwrap(),
                Some(BlockStatus::Valid)
            );
        }
    }

    #[tokio::test]
    async fn missing_tracker_parent_defers_without_invalidating_child() {
        let (genesis, mut state) = execute_test_genesis();
        let parent = execute_test_block(&mut state, &genesis.block, 1_001, 1);
        let child = execute_test_block(&mut state, &parent.block, 1_002, 2);
        let fixture = FcmTestFixture::new(&genesis, &[]);
        for block in [&parent, &child] {
            seed_executed_block(fixture.ctx.storage(), block, BlockStatus::Unchecked);
        }
        fixture
            .ctx
            .storage()
            .set_block_high_watermark(child.commitment());
        let mut fcm = fixture.fcm_state_at(empty_tracker(&genesis), &genesis);

        process_fc_message(&ForkChoiceMessage::NewBlock(child.blkid()), &mut fcm)
            .await
            .unwrap();

        assert_eq!(
            fixture.ctx.get_block_status(child.blkid()).await.unwrap(),
            Some(BlockStatus::Unchecked)
        );
        assert_eq!(
            fixture.ctx.storage().block_high_watermark(),
            Some(child.commitment())
        );
        assert!(fixture.ctx.executed_blocks().is_empty());
        assert!(!fcm.chain_tracker().is_seen_block(&child.blkid()));
        assert_eq!(fcm.pending_block_count(), 1);

        process_fc_message(&ForkChoiceMessage::NewBlock(parent.blkid()), &mut fcm)
            .await
            .unwrap();
        retry_pending_blocks(&mut fcm, true).await.unwrap();

        assert_eq!(fcm.cur_best_block(), child.commitment());
        assert_eq!(fcm.pending_block_count(), 0);
        assert_eq!(
            fixture.ctx.get_block_status(child.blkid()).await.unwrap(),
            Some(BlockStatus::Valid)
        );
        assert_eq!(
            fixture.ctx.storage().block_high_watermark(),
            Some(child.commitment())
        );
        assert_eq!(
            fixture.ctx.executed_blocks(),
            vec![parent.commitment(), child.commitment()]
        );
    }

    #[tokio::test]
    async fn child_retries_on_parent_arrival_without_waiting_for_status_scan_wrap() {
        let (genesis, mut state) = execute_test_genesis();
        let parent = execute_test_block(&mut state, &genesis.block, 1_001, 1);
        let child = execute_test_block(&mut state, &parent.block, 1_002, 2);
        let fixture = FcmTestFixture::new(&genesis, &[]);
        seed_executed_block(fixture.ctx.storage(), &child, BlockStatus::Unchecked);
        fixture.ctx.set_execution_outcome(
            child.blkid(),
            BlockExecutionOutcome::Deferred(ExecutionDeferral::Dependency),
        );
        let mut fcm = fixture.fcm_state_at(empty_tracker(&genesis), &genesis);
        fcm.record_pending_status_page(&[(child.commitment(), BlockStatus::Unchecked)]);

        // Keep both event-triggered scans beyond the child, with no wrap or rediscovery.
        let first_filler_slot = child.commitment().slot() + 1;
        for slot in first_filler_slot..first_filler_slot + 2 * STATUS_SCAN_SIZE as Slot {
            let block = make_storage_block(slot, genesis.blkid());
            let commitment = fixture.ctx.storage().put_ol_block(block);
            fixture
                .ctx
                .set_block_status(*commitment.blkid(), BlockStatus::Invalid)
                .await
                .unwrap();
        }

        <FcmService<StubFcmContext> as AsyncService>::process_input(
            &mut fcm,
            FcmEvent::NewFcmMsg(ForkChoiceMessage::NewBlock(child.blkid())),
        )
        .await
        .unwrap();

        assert!(fcm.pending_block_count() > 0);
        assert_eq!(
            fixture.ctx.get_block_status(child.blkid()).await.unwrap(),
            Some(BlockStatus::Unchecked)
        );
        assert!(fcm.pending_scan_cursor().unwrap().slot() > child.commitment().slot());

        seed_executed_block(fixture.ctx.storage(), &parent, BlockStatus::Unchecked);
        fixture.ctx.clear_execution_outcomes();
        <FcmService<StubFcmContext> as AsyncService>::process_input(
            &mut fcm,
            FcmEvent::NewFcmMsg(ForkChoiceMessage::NewBlock(parent.blkid())),
        )
        .await
        .unwrap();

        assert_eq!(fcm.cur_best_block(), child.commitment());
        assert_eq!(
            fixture.ctx.executed_blocks(),
            vec![parent.commitment(), child.commitment()]
        );
        assert_eq!(
            fixture.ctx.get_block_status(child.blkid()).await.unwrap(),
            Some(BlockStatus::Valid)
        );
        assert!(fcm.pending_scan_cursor().unwrap().slot() > child.commitment().slot());
    }

    #[tokio::test(start_paused = true)]
    async fn full_orphan_cache_does_not_prevent_honest_block_recovery() {
        let (genesis, mut state) = execute_test_genesis();
        let honest = execute_test_block(&mut state, &genesis.block, 1_001, 1);
        let fixture = FcmTestFixture::new(&genesis, &[]);
        let mut fcm = fixture.fcm_state_at(empty_tracker(&genesis), &genesis);
        for slot in 2..=257 {
            let orphan = make_storage_block(slot, OLBlockId::null());
            let id = orphan.header().compute_blkid();
            fixture.ctx.storage().put_ol_block(orphan.clone());
            fixture
                .ctx
                .set_block_status(id, BlockStatus::Unchecked)
                .await
                .unwrap();
            fcm.discover_pending_block(&orphan);
        }
        assert_eq!(fcm.pending_block_count(), 256);
        seed_executed_block(fixture.ctx.storage(), &honest, BlockStatus::Unchecked);
        fixture.ctx.set_execution_outcome(
            honest.blkid(),
            BlockExecutionOutcome::Deferred(ExecutionDeferral::Dependency),
        );
        process_fc_message(&ForkChoiceMessage::NewBlock(honest.blkid()), &mut fcm)
            .await
            .unwrap();
        fixture.ctx.clear_execution_outcomes();

        // Orphans remain cached until capacity eviction can reclaim their slots.
        advance(Duration::from_secs(1)).await;
        retry_pending_blocks(&mut fcm, true).await.unwrap();

        assert_eq!(fcm.cur_best_block(), honest.commitment());
        assert_eq!(
            fixture.ctx.get_block_status(honest.blkid()).await.unwrap(),
            Some(BlockStatus::Valid)
        );
    }

    #[tokio::test]
    async fn finalized_history_scan_does_not_load_block_bodies() {
        let (genesis, _) = execute_test_genesis();
        let fixture = FcmTestFixture::new(&genesis, &[]);
        let finalized = EpochCommitment::new(1, 100, genesis.blkid());
        let tracker = UnfinalizedBlockTracker::new_empty(finalized);
        let mut fcm = fixture.fcm_state_at(tracker, &genesis);
        for slot in 1..=100 {
            let historical = make_storage_block(slot, genesis.blkid());
            let id = historical.header().compute_blkid();
            fixture.ctx.storage().put_ol_block(historical.clone());
            fixture
                .ctx
                .set_block_status(id, BlockStatus::Valid)
                .await
                .unwrap();
            fcm.discover_pending_block(&historical);
        }
        fixture
            .ctx
            .storage()
            .inner
            .lock()
            .unwrap()
            .block_reads
            .clear();

        // Finality supersedes cursors below or at its slot, as well as a fresh scan.
        for cursor_slot in [None, Some(50), Some(100)] {
            if let Some(slot) = cursor_slot {
                let block = make_storage_block(slot, genesis.blkid());
                fcm.record_pending_status_page(&[(
                    block.header().compute_block_commitment(),
                    BlockStatus::Valid,
                )]);
            }
            retry_pending_blocks(&mut fcm, true).await.unwrap();
        }

        let inner = fixture.ctx.storage().inner.lock().unwrap();
        assert_eq!(inner.status_scans, 3);
        assert_eq!(inner.block_reads, Vec::<OLBlockId>::new());
        assert_eq!(fcm.pending_block_count(), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn full_ready_cache_preserves_unattempted_blocks_despite_age() {
        let (genesis, _) = execute_test_genesis();
        let fixture = FcmTestFixture::new(&genesis, &[]);
        let mut fcm = fixture.fcm_state_at(empty_tracker(&genesis), &genesis);
        let mut expected = Vec::new();
        for slot in 1..=256 {
            let block = make_storage_block(slot, genesis.blkid());
            expected.push(block.header().compute_blkid());
            fcm.discover_pending_block(&block);
        }

        advance(Duration::from_secs(60)).await;
        fcm.discover_pending_block(&make_storage_block(257, genesis.blkid()));
        assert_eq!(fcm.pending_block_count(), expected.len());

        // Keep the durable table empty so discovery cannot hide an eviction. Each
        // retry must read the original cached ID's status before removing it.
        for batch in 0..expected.len().div_ceil(RETRY_BATCH_SIZE) {
            <FcmService<StubFcmContext> as AsyncService>::process_input(
                &mut fcm,
                FcmEvent::RetryTick,
            )
            .await
            .unwrap();
            let attempted = ((batch + 1) * RETRY_BATCH_SIZE).min(expected.len());
            let inner = fixture.ctx.storage().inner.lock().unwrap();
            assert_eq!(inner.status_reads, expected[..attempted]);
            if batch == 0 {
                assert_eq!(inner.status_scans, 0);
            }
            assert_eq!(fcm.pending_block_count(), expected.len() - attempted);
        }
        assert_eq!(fcm.pending_block_count(), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn full_cache_preserves_storage_backoff_during_discovery_and_refill() {
        let (genesis, _) = execute_test_genesis();
        let fixture = FcmTestFixture::new(&genesis, &[]);
        let mut fcm = fixture.fcm_state_at(empty_tracker(&genesis), &genesis);
        for slot in 1..=300 {
            let block = make_storage_block(slot, genesis.blkid());
            let commitment = fixture.ctx.storage().put_ol_block(block.clone());
            fixture
                .ctx
                .set_block_status(*commitment.blkid(), BlockStatus::Unchecked)
                .await
                .unwrap();
            fixture.ctx.set_execution_outcome(
                *commitment.blkid(),
                BlockExecutionOutcome::Deferred(ExecutionDeferral::Storage),
            );
            if slot <= 256 {
                // Six failures reach the 32-second storage retry deadline.
                for _ in 0..6 {
                    fcm.defer_block(&block, ExecutionDeferral::Storage);
                }
            } else {
                // Overflow discoveries must not replace entries still backing off.
                fcm.discover_pending_block(&block);
            }
        }
        assert_eq!(fcm.pending_block_count(), 256);

        for _ in 0..31 {
            advance(Duration::from_secs(1)).await;
            for event in [FcmEvent::NewStateUpdate, FcmEvent::RetryTick] {
                <FcmService<StubFcmContext> as AsyncService>::process_input(&mut fcm, event)
                    .await
                    .unwrap();
            }
            assert!(fixture.ctx.executed_blocks().is_empty());
            assert_eq!(fixture.ctx.storage().inner.lock().unwrap().status_scans, 0);
            assert_eq!(fcm.pending_scan_cursor(), None);
            assert_eq!(fcm.pending_block_count(), 256);
        }

        advance(Duration::from_secs(1)).await;
        <FcmService<StubFcmContext> as AsyncService>::process_input(&mut fcm, FcmEvent::RetryTick)
            .await
            .unwrap();

        assert!(!fixture.ctx.executed_blocks().is_empty());
        assert_eq!(fixture.ctx.storage().inner.lock().unwrap().status_scans, 1);
        assert!(fcm.pending_block_count() <= 256);
    }

    #[tokio::test(start_paused = true)]
    async fn durable_refill_keeps_cursor_before_failed_body_read() {
        let (genesis, _) = execute_test_genesis();
        let fixture = FcmTestFixture::new(&genesis, &[]);
        let mut candidates = Vec::new();
        for slot in 1..=130 {
            let block = make_storage_block(slot, genesis.blkid());
            let commitment = fixture.ctx.storage().put_ol_block(block);
            fixture
                .ctx
                .set_block_status(*commitment.blkid(), BlockStatus::Unchecked)
                .await
                .unwrap();
            fixture.ctx.set_execution_outcome(
                *commitment.blkid(),
                BlockExecutionOutcome::Deferred(ExecutionDeferral::Dependency),
            );
            candidates.push(commitment);
        }
        let failed = candidates[1];
        fixture
            .ctx
            .storage()
            .inner
            .lock()
            .unwrap()
            .body_read_failures
            .insert(*failed.blkid());
        let mut fcm = fixture.fcm_state_at(empty_tracker(&genesis), &genesis);

        <FcmService<StubFcmContext> as AsyncService>::process_input(&mut fcm, FcmEvent::RetryTick)
            .await
            .unwrap();
        assert_eq!(fcm.pending_scan_cursor(), Some(candidates[0]));
        assert_eq!(fixture.ctx.executed_blocks(), vec![candidates[0]]);

        advance(Duration::from_secs(1)).await;
        <FcmService<StubFcmContext> as AsyncService>::process_input(&mut fcm, FcmEvent::RetryTick)
            .await
            .unwrap();
        assert!(fixture.ctx.executed_blocks().contains(&failed));
        assert_eq!(
            fcm.pending_scan_cursor(),
            Some(candidates[STATUS_SCAN_SIZE])
        );
    }

    #[tokio::test(start_paused = true)]
    async fn durable_refill_skips_persistent_body_failure_and_revisits_after_wrap() {
        let (genesis, _) = execute_test_genesis();
        let fixture = FcmTestFixture::new(&genesis, &[]);
        let mut candidates = Vec::new();
        for slot in 1..=3 {
            let block = make_storage_block(slot, genesis.blkid());
            let commitment = fixture.ctx.storage().put_ol_block(block);
            fixture
                .ctx
                .set_block_status(*commitment.blkid(), BlockStatus::Unchecked)
                .await
                .unwrap();
            fixture.ctx.set_execution_outcome(
                *commitment.blkid(),
                BlockExecutionOutcome::Deferred(ExecutionDeferral::Dependency),
            );
            candidates.push(commitment);
        }
        let failed = candidates[1];
        let mut fcm = fixture.fcm_state_at(empty_tracker(&genesis), &genesis);

        for _ in 0..3 {
            fixture
                .ctx
                .storage()
                .inner
                .lock()
                .unwrap()
                .body_read_failures
                .insert(*failed.blkid());
            <FcmService<StubFcmContext> as AsyncService>::process_input(
                &mut fcm,
                FcmEvent::RetryTick,
            )
            .await
            .unwrap();
            advance(Duration::from_secs(1)).await;
        }

        assert!(!fixture.ctx.executed_blocks().contains(&failed));
        assert!(fixture.ctx.executed_blocks().contains(&candidates[2]));
        assert_eq!(fcm.pending_scan_cursor(), Some(candidates[2]));

        // The failed body becomes readable while discovery wraps through durable metadata.
        for _ in 0..2 {
            <FcmService<StubFcmContext> as AsyncService>::process_input(
                &mut fcm,
                FcmEvent::RetryTick,
            )
            .await
            .unwrap();
            advance(Duration::from_secs(1)).await;
        }
        assert!(fixture.ctx.executed_blocks().contains(&failed));
        assert_eq!(
            fixture.ctx.get_block_status(*failed.blkid()).await.unwrap(),
            Some(BlockStatus::Unchecked)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn durable_refill_retries_overflow_without_restart() {
        let (genesis, _) = execute_test_genesis();
        let fixture = FcmTestFixture::new(&genesis, &[]);
        let mut expected = BTreeSet::new();
        for slot in 1..=320 {
            let block = make_storage_block(slot, genesis.blkid());
            let commitment = fixture.ctx.storage().put_ol_block(block);
            fixture
                .ctx
                .set_block_status(*commitment.blkid(), BlockStatus::Unchecked)
                .await
                .unwrap();
            fixture.ctx.set_execution_outcome(
                *commitment.blkid(),
                BlockExecutionOutcome::Deferred(ExecutionDeferral::Dependency),
            );
            expected.insert(*commitment.blkid());
        }
        let mut fcm = fixture.fcm_state_at(empty_tracker(&genesis), &genesis);
        let mut attempted = BTreeSet::new();
        // Allow room for cache rotation as well as one retry batch per entry.
        let max_retry_cycles = 2 * expected.len().div_ceil(RETRY_BATCH_SIZE);
        for _ in 0..max_retry_cycles {
            advance(Duration::from_secs(1)).await;
            <FcmService<StubFcmContext> as AsyncService>::process_input(
                &mut fcm,
                FcmEvent::RetryTick,
            )
            .await
            .unwrap();
            assert!(fcm.pending_block_count() <= 256);
            attempted.extend(
                fixture
                    .ctx
                    .executed_blocks()
                    .iter()
                    .map(|block| *block.blkid()),
            );
            if attempted == expected {
                break;
            }
        }
        assert_eq!(
            attempted.len(),
            expected.len(),
            "all durable entries must eventually get a turn"
        );
        assert_eq!(attempted, expected);
        assert_eq!(fcm.cur_best_block(), genesis.commitment());
    }

    #[tokio::test]
    async fn deferred_execution_preserves_status_head_and_high_watermark() {
        let (genesis, mut state) = execute_test_genesis();
        let block = execute_test_block(&mut state, &genesis.block, 1_001, 1);
        let fixture = FcmTestFixture::new(&genesis, &[]);
        fixture.ctx.storage().put_ol_block(block.block.clone());
        fixture
            .ctx
            .storage()
            .set_block_status(block.blkid(), BlockStatus::Unchecked)
            .await
            .unwrap();
        fixture
            .ctx
            .storage()
            .set_block_high_watermark(block.commitment());
        let mut fcm = fixture.fcm_state_at(empty_tracker(&genesis), &genesis);
        for reason in [ExecutionDeferral::Dependency, ExecutionDeferral::Storage] {
            fixture
                .ctx
                .set_execution_outcome(block.blkid(), BlockExecutionOutcome::Deferred(reason));
            process_fc_message(&ForkChoiceMessage::NewBlock(block.blkid()), &mut fcm)
                .await
                .unwrap();
            assert_eq!(
                fixture.ctx.get_block_status(block.blkid()).await.unwrap(),
                Some(BlockStatus::Unchecked)
            );
            assert_eq!(fcm.cur_best_block(), genesis.commitment());
            assert_eq!(
                fixture.ctx.storage().block_high_watermark(),
                Some(block.commitment())
            );
            assert!(fixture.ctx.storage().indexing_rollbacks().is_empty());
            assert!(fixture.ctx.storage().epoch_summary_deletes().is_empty());
            assert!(fixture.ctx.published_statuses().is_empty());
        }
    }

    #[tokio::test(start_paused = true)]
    async fn retry_preserves_durable_verdict_and_indexing_on_local_failure() {
        for status in [BlockStatus::Unchecked, BlockStatus::Valid] {
            for deferral in [
                Some(ExecutionDeferral::Storage),
                Some(ExecutionDeferral::Dependency),
                None,
            ] {
                let (genesis, mut state) = execute_test_genesis();
                let block =
                    execute_terminal_test_block_in_epoch(&mut state, &genesis.block, 1_001, 1, 1);
                let fixture = FcmTestFixture::new(&genesis, &[]);
                seed_executed_block(fixture.ctx.storage(), &block, status);
                fixture
                    .ctx
                    .storage()
                    .set_block_high_watermark(block.commitment());
                fixture
                    .ctx
                    .storage()
                    .put_canonical_epoch_commitment(EpochCommitment::new(1, 1, block.blkid()));
                if let Some(reason) = deferral {
                    fixture.ctx.set_execution_outcome(
                        block.blkid(),
                        BlockExecutionOutcome::Deferred(reason),
                    );
                } else {
                    fixture
                        .ctx
                        .execution_errors
                        .lock()
                        .unwrap()
                        .insert(block.blkid());
                }
                let mut fcm = fixture.fcm_state_at(empty_tracker(&genesis), &genesis);

                let result = <FcmService<StubFcmContext> as AsyncService>::process_input(
                    &mut fcm,
                    FcmEvent::RetryTick,
                )
                .await;
                if deferral.is_some() {
                    result.unwrap();
                } else {
                    assert!(result
                        .unwrap_err()
                        .to_string()
                        .contains("injected unclassified worker failure"));
                }

                // The verdict must remain intact during execution as well as after deferral.
                assert_eq!(
                    *fixture.ctx.execution_statuses.lock().unwrap(),
                    vec![Some(status)]
                );
                assert_eq!(
                    fixture.ctx.get_block_status(block.blkid()).await.unwrap(),
                    Some(status)
                );
                assert_eq!(
                    fixture.ctx.storage().block_high_watermark(),
                    Some(block.commitment())
                );
                assert_eq!(fcm.cur_best_block(), genesis.commitment());
                assert_eq!(fcm.pending_block_count(), 1);
                assert!(fixture.ctx.storage().indexing_rollbacks().is_empty());
                assert!(fixture.ctx.storage().epoch_summary_deletes().is_empty());
                assert!(fixture.ctx.published_statuses().is_empty());

                if deferral.is_none() {
                    // A fatal local failure terminates the service and preserves durable data.
                    continue;
                }
                if deferral != Some(ExecutionDeferral::Dependency) {
                    <FcmService<StubFcmContext> as AsyncService>::process_input(
                        &mut fcm,
                        FcmEvent::NewStateUpdate,
                    )
                    .await
                    .unwrap();
                    assert_eq!(fixture.ctx.executed_blocks(), vec![block.commitment()]);
                }

                fixture.ctx.clear_execution_outcomes();
                fixture.ctx.execution_errors.lock().unwrap().clear();
                advance(Duration::from_secs(1)).await;
                <FcmService<StubFcmContext> as AsyncService>::process_input(
                    &mut fcm,
                    FcmEvent::RetryTick,
                )
                .await
                .unwrap();

                assert_eq!(
                    fixture.ctx.get_block_status(block.blkid()).await.unwrap(),
                    Some(BlockStatus::Valid)
                );
                assert_eq!(fcm.cur_best_block(), block.commitment());
                assert_eq!(fcm.pending_block_count(), 0);
                assert_eq!(
                    fixture.ctx.storage().block_high_watermark(),
                    Some(block.commitment())
                );
                assert!(fixture.ctx.storage().indexing_rollbacks().is_empty());
                assert!(fixture.ctx.storage().epoch_summary_deletes().is_empty());
            }
        }
    }

    #[tokio::test]
    async fn fatal_execution_failure_propagates_from_new_block_and_startup() {
        for startup in [false, true] {
            let (genesis, mut state) = execute_test_genesis();
            let block =
                execute_terminal_test_block_in_epoch(&mut state, &genesis.block, 1_001, 1, 1);
            let fixture = FcmTestFixture::new(&genesis, &[]);
            seed_executed_block(fixture.ctx.storage(), &block, BlockStatus::Unchecked);
            fixture
                .ctx
                .storage()
                .set_block_high_watermark(block.commitment());
            fixture
                .ctx
                .execution_errors
                .lock()
                .unwrap()
                .insert(block.blkid());
            let inner = FcmInnerState::new(
                empty_tracker(&genesis),
                genesis.commitment(),
                Arc::new(genesis.state.clone()),
                if startup {
                    vec![block.blkid()]
                } else {
                    Vec::new()
                },
            );
            let mut fcm =
                FcmServiceState::new(fixture.ctx.clone(), PredicateKey::always_accept(), inner);

            let error = if startup {
                <FcmService<StubFcmContext> as AsyncService>::on_launch(&mut fcm)
                    .await
                    .unwrap_err()
            } else {
                <FcmService<StubFcmContext> as AsyncService>::process_input(
                    &mut fcm,
                    FcmEvent::NewFcmMsg(ForkChoiceMessage::NewBlock(block.blkid())),
                )
                .await
                .unwrap_err()
            };

            assert!(error
                .to_string()
                .contains("injected unclassified worker failure"));
            assert_eq!(fixture.ctx.executed_blocks(), vec![block.commitment()]);
            assert_eq!(
                fixture.ctx.get_block_status(block.blkid()).await.unwrap(),
                Some(BlockStatus::Unchecked)
            );
            assert_eq!(
                fixture.ctx.storage().block_high_watermark(),
                Some(block.commitment())
            );
            assert_eq!(fcm.cur_best_block(), genesis.commitment());
            assert_eq!(fcm.pending_block_count(), usize::from(startup));
            assert!(fixture.ctx.storage().indexing_rollbacks().is_empty());
            assert!(fixture.ctx.storage().epoch_summary_deletes().is_empty());
            assert!(fixture.ctx.published_statuses().is_empty());
        }
    }

    #[tokio::test(start_paused = true)]
    async fn attached_block_retries_fork_choice_without_reexecution() {
        enum Failure {
            HeaderRead,
            SafeTip,
            CanonicalWrite,
            ExhaustedStateRead,
        }
        for failure in [
            Failure::HeaderRead,
            Failure::SafeTip,
            Failure::CanonicalWrite,
            Failure::ExhaustedStateRead,
        ] {
            let (genesis, mut state) = execute_test_genesis();
            let block = execute_test_block(&mut state, &genesis.block, 1_001, 1);
            let fixture = FcmTestFixture::new(&genesis, &[]);
            seed_executed_block(fixture.ctx.storage(), &block, BlockStatus::Unchecked);
            match failure {
                Failure::HeaderRead => {
                    fixture
                        .ctx
                        .storage()
                        .inner
                        .lock()
                        .unwrap()
                        .header_read_failures
                        .insert(block.blkid());
                }
                Failure::ExhaustedStateRead => {
                    fixture
                        .ctx
                        .storage()
                        .inner
                        .lock()
                        .unwrap()
                        .state_read_errors
                        .insert(
                            block.commitment(),
                            DbError::RetriesExhausted {
                                attempts: 3,
                                last_error: Box::new(DbError::Busy),
                            },
                        );
                }
                Failure::SafeTip => *fixture.ctx.safe_tip_failures.lock().unwrap() = 1,
                Failure::CanonicalWrite => {
                    fixture
                        .ctx
                        .storage()
                        .inner
                        .lock()
                        .unwrap()
                        .canonical_write_failures = 1;
                }
            }
            let mut fcm = fixture.fcm_state_at(empty_tracker(&genesis), &genesis);
            <FcmService<StubFcmContext> as AsyncService>::process_input(
                &mut fcm,
                FcmEvent::NewFcmMsg(ForkChoiceMessage::NewBlock(block.blkid())),
            )
            .await
            .unwrap();

            assert_eq!(
                fixture.ctx.get_block_status(block.blkid()).await.unwrap(),
                Some(BlockStatus::Valid)
            );
            assert!(fcm.chain_tracker().is_seen_block(&block.blkid()));
            assert_eq!(fcm.pending_block_count(), 0);
            assert_eq!(fcm.cur_best_block(), genesis.commitment());
            assert_eq!(
                fixture
                    .ctx
                    .storage()
                    .get_canonical_block_at(1)
                    .await
                    .unwrap(),
                None
            );
            assert!(fixture.ctx.published_statuses().is_empty());

            // Progress must not bypass fork choice's storage backoff.
            <FcmService<StubFcmContext> as AsyncService>::process_input(
                &mut fcm,
                FcmEvent::NewStateUpdate,
            )
            .await
            .unwrap();
            assert_eq!(fcm.cur_best_block(), genesis.commitment());
            advance(Duration::from_secs(1)).await;
            <FcmService<StubFcmContext> as AsyncService>::process_input(
                &mut fcm,
                FcmEvent::RetryTick,
            )
            .await
            .unwrap();

            assert_eq!(fcm.cur_best_block(), block.commitment());
            assert_eq!(
                fixture
                    .ctx
                    .storage()
                    .get_canonical_block_at(1)
                    .await
                    .unwrap(),
                Some(block.commitment())
            );
            assert_eq!(
                fixture.ctx.safe_tip_updates().last(),
                Some(&block.commitment())
            );
            assert_eq!(fixture.ctx.executed_blocks(), vec![block.commitment()]);
            assert_eq!(fixture.ctx.published_statuses().len(), 1);
            assert_eq!(
                fixture.ctx.published_statuses()[0].tip(),
                block.commitment()
            );
        }
    }

    #[tokio::test]
    async fn fork_choice_propagates_missing_state_after_partial_tip_update() {
        for replay in [false, true] {
            let chain = LinearChain::new();
            let fixture = chain.fixture_without_x4();
            seed_executed_block(fixture.ctx.storage(), &chain.x4, BlockStatus::Unchecked);
            fixture
                .ctx
                .storage()
                .inner
                .lock()
                .unwrap()
                .states
                .remove(&chain.x2.commitment());
            let mut fcm = fixture.fcm_state_at(chain.tracker_through_x3(), &chain.genesis);
            let input = if replay {
                FcmEvent::RetryTick
            } else {
                FcmEvent::NewFcmMsg(ForkChoiceMessage::NewBlock(chain.x4.blkid()))
            };

            let error =
                <FcmService<StubFcmContext> as AsyncService>::process_input(&mut fcm, input)
                    .await
                    .unwrap_err();

            assert!(
                matches!(error.downcast_ref::<Error>(), Some(Error::MissingOLState(block))
                if *block == chain.x2.commitment())
            );
            assert_eq!(fixture.ctx.safe_tip_updates(), vec![chain.x1.commitment()]);
            assert_eq!(fcm.cur_best_block(), chain.genesis.commitment());
            assert_eq!(
                fixture
                    .ctx
                    .get_block_status(chain.x4.blkid())
                    .await
                    .unwrap(),
                Some(BlockStatus::Valid)
            );
            assert!(fixture.ctx.storage().indexing_rollbacks().is_empty());
            assert!(fixture.ctx.published_statuses().is_empty());
        }
    }

    #[tokio::test]
    async fn fork_choice_propagates_permanent_database_failures_during_acceptance() {
        for replay in [false, true] {
            for failure in [
                DbError::CodecError("corrupt stored state".into()),
                DbError::Other("unclassified storage failure".into()),
                DbError::OverwriteEpoch(EpochCommitment::null()),
                DbError::RetriesExhausted {
                    attempts: 3,
                    last_error: Box::new(DbError::CodecError("corrupt stored state".into())),
                },
            ] {
                let chain = LinearChain::new();
                let fixture = chain.fixture_without_x4();
                seed_executed_block(fixture.ctx.storage(), &chain.x4, BlockStatus::Unchecked);
                let expected = failure.to_string();
                fixture
                    .ctx
                    .storage()
                    .inner
                    .lock()
                    .unwrap()
                    .state_read_errors
                    .insert(chain.x2.commitment(), failure);
                let mut fcm = fixture.fcm_state_at(chain.tracker_through_x3(), &chain.genesis);
                let input = if replay {
                    FcmEvent::RetryTick
                } else {
                    FcmEvent::NewFcmMsg(ForkChoiceMessage::NewBlock(chain.x4.blkid()))
                };

                let error =
                    <FcmService<StubFcmContext> as AsyncService>::process_input(&mut fcm, input)
                        .await
                        .unwrap_err();

                assert_eq!(
                    error.downcast_ref::<DbError>().unwrap().to_string(),
                    expected
                );
                assert_eq!(fcm.cur_best_block(), chain.genesis.commitment());
                assert_eq!(
                    fixture
                        .ctx
                        .get_block_status(chain.x4.blkid())
                        .await
                        .unwrap(),
                    Some(BlockStatus::Valid)
                );
                assert!(fixture.ctx.published_statuses().is_empty());
            }
        }
    }

    #[tokio::test]
    async fn fork_choice_propagates_parent_mismatch_and_restores_original_tip() {
        let fork = TestFork::new();
        let fixture = fork.fixture();
        let mut tracker = tracker_with_blocks(&fork.genesis, &[&fork.a1]);
        tracker
            .attach_block(
                fork.a2.block.header().slot(),
                fork.a2.blkid(),
                fork.genesis.blkid(),
            )
            .unwrap();
        let mut fcm = fixture.fcm_state_at(tracker, &fork.a1);
        fcm.mark_fork_choice_pending();

        let error = retry_fork_choice(&mut fcm).await.unwrap_err();

        assert!(matches!(error.downcast_ref::<Error>(),
            Some(Error::OLApplyBlockParentMismatch(block, expected_parent, got_parent))
                if *block == fork.a2.commitment()
                    && *expected_parent == fork.genesis.commitment()
                    && *got_parent == fork.a1.blkid()));
        assert_eq!(fcm.cur_best_block(), fork.a1.commitment());
        assert!(fixture.ctx.published_statuses().is_empty());
    }

    #[tokio::test]
    async fn fatal_safe_tip_failure_propagates_at_startup_and_on_retry() {
        for startup in [false, true] {
            let (genesis, _) = execute_test_genesis();
            let fixture = FcmTestFixture::new(&genesis, &[]);
            let mut fcm = fixture.fcm_state_at(empty_tracker(&genesis), &genesis);
            *fixture.ctx.safe_tip_error.lock().unwrap() = Some(anyhow!("worker stopped"));
            let error = if startup {
                <FcmService<StubFcmContext> as AsyncService>::on_launch(&mut fcm)
                    .await
                    .unwrap_err()
            } else {
                fcm.mark_fork_choice_pending();
                <FcmService<StubFcmContext> as AsyncService>::process_input(
                    &mut fcm,
                    FcmEvent::RetryTick,
                )
                .await
                .unwrap_err()
            };
            assert_eq!(error.to_string(), "worker stopped");
            assert_eq!(fcm.cur_best_block(), genesis.commitment());
            assert!(fixture.ctx.published_statuses().is_empty());
        }
    }

    #[tokio::test(start_paused = true)]
    async fn fork_choice_retries_entire_path_after_partial_tip_update() {
        let chain = LinearChain::new();
        let fixture = chain.fixture_without_x4();
        seed_executed_block(fixture.ctx.storage(), &chain.x4, BlockStatus::Unchecked);
        fixture
            .ctx
            .storage()
            .inner
            .lock()
            .unwrap()
            .state_read_errors
            .insert(chain.x2.commitment(), DbError::Busy);
        let mut fcm = fixture.fcm_state_at(chain.tracker_through_x3(), &chain.genesis);

        <FcmService<StubFcmContext> as AsyncService>::process_input(
            &mut fcm,
            FcmEvent::NewFcmMsg(ForkChoiceMessage::NewBlock(chain.x4.blkid())),
        )
        .await
        .unwrap();

        assert_eq!(fixture.ctx.safe_tip_updates(), vec![chain.x1.commitment()]);
        assert_eq!(fcm.cur_best_block(), chain.genesis.commitment());
        assert_eq!(fcm.pending_block_count(), 0);
        assert_eq!(
            fixture
                .ctx
                .storage()
                .get_canonical_block_at(1)
                .await
                .unwrap(),
            None
        );

        advance(Duration::from_secs(1)).await;
        <FcmService<StubFcmContext> as AsyncService>::process_input(&mut fcm, FcmEvent::RetryTick)
            .await
            .unwrap();

        assert_eq!(fcm.cur_best_block(), chain.x4.commitment());
        for block in [&chain.x1, &chain.x2, &chain.x3, &chain.x4] {
            assert_eq!(
                fixture
                    .ctx
                    .storage()
                    .get_canonical_block_at(block.commitment().slot())
                    .await
                    .unwrap(),
                Some(block.commitment())
            );
        }
        assert_eq!(
            fixture.ctx.safe_tip_updates().last(),
            Some(&chain.x4.commitment())
        );
        assert_eq!(fixture.ctx.executed_blocks(), vec![chain.x4.commitment()]);
    }

    #[tokio::test(start_paused = true)]
    async fn child_waits_for_parent_after_temporary_status_write_failure() {
        let (genesis, mut state) = execute_test_genesis();
        let parent = execute_test_block(&mut state, &genesis.block, 1_001, 1);
        let child = execute_test_block(&mut state, &parent.block, 1_002, 2);
        let fixture = FcmTestFixture::new(&genesis, &[]);
        // Execution artifacts are already readable, as they would be after the worker succeeds.
        seed_executed_block(fixture.ctx.storage(), &parent, BlockStatus::Unchecked);
        let mut fcm = fixture.fcm_state_at(empty_tracker(&genesis), &genesis);
        fixture
            .ctx
            .storage()
            .inner
            .lock()
            .unwrap()
            .status_writes_until_failure = Some(0);

        <FcmService<StubFcmContext> as AsyncService>::process_input(
            &mut fcm,
            FcmEvent::NewFcmMsg(ForkChoiceMessage::NewBlock(parent.blkid())),
        )
        .await
        .unwrap();
        assert_eq!(fixture.ctx.executed_blocks(), vec![parent.commitment()]);
        assert!(!fcm.chain_tracker().is_seen_block(&parent.blkid()));
        assert_eq!(fcm.pending_block_count(), 1);

        seed_executed_block(fixture.ctx.storage(), &child, BlockStatus::Unchecked);
        <FcmService<StubFcmContext> as AsyncService>::process_input(
            &mut fcm,
            FcmEvent::NewFcmMsg(ForkChoiceMessage::NewBlock(child.blkid())),
        )
        .await
        .unwrap();
        assert_eq!(fixture.ctx.executed_blocks(), vec![parent.commitment()]);
        for block in [&parent, &child] {
            assert_eq!(
                fixture.ctx.get_block_status(block.blkid()).await.unwrap(),
                Some(BlockStatus::Unchecked)
            );
        }
        assert_eq!(fcm.cur_best_block(), genesis.commitment());
        assert_eq!(fcm.pending_block_count(), 2);

        advance(Duration::from_secs(1)).await;
        <FcmService<StubFcmContext> as AsyncService>::process_input(&mut fcm, FcmEvent::RetryTick)
            .await
            .unwrap();
        assert_eq!(
            fixture.ctx.executed_blocks(),
            vec![parent.commitment(), parent.commitment(), child.commitment()]
        );
        for block in [&parent, &child] {
            assert_eq!(
                fixture.ctx.get_block_status(block.blkid()).await.unwrap(),
                Some(BlockStatus::Valid)
            );
        }
        assert_eq!(fcm.cur_best_block(), child.commitment());
        assert_eq!(fcm.pending_block_count(), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn acceptance_recovers_status_write_failure_without_redundant_writes() {
        for successful_writes in [0, 1] {
            let (genesis, mut state) = execute_test_genesis();
            let block = execute_test_block(&mut state, &genesis.block, 1_001, 1);
            let fixture = FcmTestFixture::new(&genesis, &[]);
            seed_executed_block(fixture.ctx.storage(), &block, BlockStatus::Unchecked);
            fixture
                .ctx
                .storage()
                .inner
                .lock()
                .unwrap()
                .status_writes_until_failure = Some(successful_writes);
            let mut fcm = fixture.fcm_state_at(empty_tracker(&genesis), &genesis);

            <FcmService<StubFcmContext> as AsyncService>::process_input(
                &mut fcm,
                FcmEvent::NewFcmMsg(ForkChoiceMessage::NewBlock(block.blkid())),
            )
            .await
            .unwrap();
            if successful_writes == 0 {
                assert_eq!(fcm.cur_best_block(), genesis.commitment());
                assert_eq!(fcm.pending_block_count(), 1);
                advance(Duration::from_secs(1)).await;
                <FcmService<StubFcmContext> as AsyncService>::process_input(
                    &mut fcm,
                    FcmEvent::RetryTick,
                )
                .await
                .unwrap();
            }
            // Once Valid is durable, advancing fork choice needs no further status write.
            assert_eq!(fcm.cur_best_block(), block.commitment());
            assert_eq!(fcm.pending_block_count(), 0);
            assert_eq!(
                fixture.ctx.get_block_status(block.blkid()).await.unwrap(),
                Some(BlockStatus::Valid)
            );
            assert_eq!(
                fixture
                    .ctx
                    .storage()
                    .get_canonical_block_at(1)
                    .await
                    .unwrap(),
                Some(block.commitment())
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn new_block_status_read_failure_uses_storage_backoff() {
        let (genesis, mut state) = execute_test_genesis();
        let block = execute_test_block(&mut state, &genesis.block, 1_001, 1);
        let fixture = FcmTestFixture::new(&genesis, &[]);
        seed_executed_block(fixture.ctx.storage(), &block, BlockStatus::Unchecked);
        fixture
            .ctx
            .storage()
            .inner
            .lock()
            .unwrap()
            .status_read_failures
            .insert(block.blkid());
        let mut fcm = fixture.fcm_state_at(empty_tracker(&genesis), &genesis);

        <FcmService<StubFcmContext> as AsyncService>::process_input(
            &mut fcm,
            FcmEvent::NewFcmMsg(ForkChoiceMessage::NewBlock(block.blkid())),
        )
        .await
        .unwrap();
        assert_eq!(fcm.pending_block_count(), 1);
        assert!(fixture.ctx.executed_blocks().is_empty());
        <FcmService<StubFcmContext> as AsyncService>::process_input(
            &mut fcm,
            FcmEvent::NewStateUpdate,
        )
        .await
        .unwrap();
        assert!(fixture.ctx.executed_blocks().is_empty());

        advance(Duration::from_secs(1)).await;
        <FcmService<StubFcmContext> as AsyncService>::process_input(&mut fcm, FcmEvent::RetryTick)
            .await
            .unwrap();
        assert_eq!(fcm.cur_best_block(), block.commitment());
        assert_eq!(fixture.ctx.executed_blocks(), vec![block.commitment()]);
        assert_eq!(fcm.pending_block_count(), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn rejection_cleanup_retries_without_execution_or_parent_dependency() {
        for failure in [
            CleanupFailure::Summary,
            CleanupFailure::WatermarkRead,
            CleanupFailure::Rollback,
            CleanupFailure::WatermarkClear,
            CleanupFailure::CompletionWrite,
        ] {
            for (finalized_slot, restart) in [
                (0, false),
                (0, true),
                (1, false),
                (1, true),
                (2, false),
                (2, true),
            ] {
                let (genesis, _) = execute_test_genesis();
                let fixture = FcmTestFixture::new(&genesis, &[]);
                // Resume a durable rejection whose parent is no longer available to attach.
                let block = make_terminal_storage_block(1, OLBlockId::null());
                let commitment = fixture.ctx.storage().put_ol_block(block);
                fixture
                    .ctx
                    .set_block_status(*commitment.blkid(), BlockStatus::Invalid)
                    .await
                    .unwrap();
                fixture.ctx.storage().set_block_high_watermark(commitment);
                fixture.ctx.storage().inner.lock().unwrap().cleanup_failure = Some(failure);
                let mut fcm = fixture.fcm_state_at(empty_tracker(&genesis), &genesis);
                assert!(!fcm.chain_tracker().is_seen_block(&OLBlockId::null()));

                <FcmService<StubFcmContext> as AsyncService>::process_input(
                    &mut fcm,
                    FcmEvent::NewFcmMsg(ForkChoiceMessage::NewBlock(*commitment.blkid())),
                )
                .await
                .unwrap();
                assert_eq!(
                    fixture
                        .ctx
                        .get_block_status(*commitment.blkid())
                        .await
                        .unwrap(),
                    Some(BlockStatus::Invalid)
                );
                assert_eq!(
                    fixture.ctx.storage().block_high_watermark(),
                    (failure != CleanupFailure::CompletionWrite).then_some(commitment)
                );
                assert_eq!(fcm.pending_block_count(), 1);
                assert!(fixture.ctx.executed_blocks().is_empty());

                <FcmService<StubFcmContext> as AsyncService>::process_input(
                    &mut fcm,
                    FcmEvent::NewStateUpdate,
                )
                .await
                .unwrap();
                assert_eq!(
                    fixture.ctx.storage().block_high_watermark(),
                    (failure != CleanupFailure::CompletionWrite).then_some(commitment)
                );
                if restart {
                    // The Invalid verdict must recover cleanup even without the old cache.
                    fcm =
                        init_fcm_service_state(PredicateKey::always_accept(), fixture.ctx.clone())
                            .await
                            .unwrap();
                }
                // Finality can overtake failed cleanup, including across a restart that loses
                // the pending cache and must rediscover the rejected block from storage.
                let finalized = EpochCommitment::new(1, finalized_slot, genesis.blkid());
                *fcm.chain_tracker_mut() = UnfinalizedBlockTracker::new_empty(finalized);
                advance(Duration::from_secs(1)).await;
                <FcmService<StubFcmContext> as AsyncService>::process_input(
                    &mut fcm,
                    FcmEvent::RetryTick,
                )
                .await
                .unwrap();

                assert_eq!(fixture.ctx.storage().block_high_watermark(), None);
                assert_eq!(fcm.pending_block_count(), 0);
                assert!(fixture.ctx.executed_blocks().is_empty());
                assert!(!fixture.ctx.storage().indexing_rollbacks().is_empty());
                assert!(!fixture.ctx.storage().epoch_summary_deletes().is_empty());
                assert!(fixture
                    .ctx
                    .is_block_rejection_complete(*commitment.blkid())
                    .await
                    .unwrap());

                // Completion survives restart and repeated wraps without loading invalid bodies
                // or consuming the retry budget again.
                fcm = init_fcm_service_state(PredicateKey::always_accept(), fixture.ctx.clone())
                    .await
                    .unwrap();
                *fcm.chain_tracker_mut() = UnfinalizedBlockTracker::new_empty(finalized);
                let cleanup_counts = {
                    let mut inner = fixture.ctx.storage().inner.lock().unwrap();
                    inner.block_reads.clear();
                    inner.status_writes_until_failure = Some(0);
                    (
                        inner.indexing_rollbacks.len(),
                        inner.epoch_summary_deletes.len(),
                    )
                };
                for _ in 0..6 {
                    advance(Duration::from_secs(1)).await;
                    <FcmService<StubFcmContext> as AsyncService>::process_input(
                        &mut fcm,
                        FcmEvent::RetryTick,
                    )
                    .await
                    .unwrap();
                    assert_eq!(fcm.pending_block_count(), 0);
                }
                let inner = fixture.ctx.storage().inner.lock().unwrap();
                assert!(inner.block_reads.is_empty());
                assert_eq!(inner.status_writes_until_failure, Some(0));
                assert_eq!(
                    (
                        inner.indexing_rollbacks.len(),
                        inner.epoch_summary_deletes.len()
                    ),
                    cleanup_counts
                );
            }
        }
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

    #[tokio::test]
    async fn permanent_storage_errors_propagate_from_recovery_boundaries() {
        for operation in [
            StorageOperation::StatusWrite,
            StorageOperation::StatusRead,
            StorageOperation::BodyRead,
            StorageOperation::StatusScan,
            StorageOperation::RejectionScan,
            StorageOperation::Cleanup,
        ] {
            for wrapped in [false, true] {
                let (genesis, mut state) = execute_test_genesis();
                let block =
                    execute_terminal_test_block_in_epoch(&mut state, &genesis.block, 1_001, 1, 1);
                let fixture = FcmTestFixture::new(&genesis, &[]);
                seed_executed_block(fixture.ctx.storage(), &block, BlockStatus::Unchecked);
                let mut fcm = fixture.fcm_state_at(empty_tracker(&genesis), &genesis);
                let error = DbError::CodecError("corrupt stored block".into());
                let error = if wrapped {
                    DbError::RetriesExhausted {
                        attempts: 2,
                        last_error: Box::new(error),
                    }
                } else {
                    error
                };
                let expected = error.to_string();
                fixture.ctx.storage().inner.lock().unwrap().storage_error =
                    Some((operation, error));

                let result = match operation {
                    StorageOperation::StatusWrite => {
                        process_fc_message(&ForkChoiceMessage::NewBlock(block.blkid()), &mut fcm)
                            .await
                    }
                    StorageOperation::StatusRead => {
                        fcm.discover_pending_block(&block.block);
                        retry_pending_blocks(&mut fcm, true).await
                    }
                    StorageOperation::BodyRead | StorageOperation::StatusScan => {
                        refill_pending_blocks(&mut fcm, STATUS_SCAN_SIZE).await
                    }
                    StorageOperation::RejectionScan => {
                        refill_pending_rejections(&mut fcm, STATUS_SCAN_SIZE).await
                    }
                    StorageOperation::Cleanup => finish_rejection(&mut fcm, &block.block).await,
                };
                let error =
                    result.expect_err("permanent storage errors must reach the service monitor");
                assert_eq!(
                    error.downcast_ref::<DbError>().unwrap().to_string(),
                    expected
                );
                assert_eq!(
                    fixture.ctx.get_block_status(block.blkid()).await.unwrap(),
                    Some(BlockStatus::Unchecked)
                );
                assert_eq!(fcm.cur_best_block(), genesis.commitment());
                assert!(fixture.ctx.storage().indexing_rollbacks().is_empty());
            }
        }
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
