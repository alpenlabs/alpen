//! Service state and checkpoint application logic for the checkpoint sync service.

use std::sync::Arc;

use strata_params::{is_l1_reorg_safe, l1_confirmations};
use strata_primitives::EpochCommitment;
use strata_service::ServiceState;
use strata_status::OLSyncStatus;
use tracing::{debug, info};

use crate::checkpoint_sync::{
    context::CheckpointSyncCtx,
    errors::{CheckpointSyncError, CheckpointSyncResult},
};

/// Service state for the checkpoint sync service.
#[derive(Debug, Clone)]
pub struct CheckpointSyncState<C: CheckpointSyncCtx> {
    /// Dependency context.
    ctx: Arc<C>,
    /// Last epoch that has been both finalized and applied to OL state.
    last_finalized_and_applied: Option<EpochCommitment>,
}

impl<C: CheckpointSyncCtx> CheckpointSyncState<C> {
    pub(crate) fn new(ctx: Arc<C>, last_finalized_and_applied: Option<EpochCommitment>) -> Self {
        Self {
            ctx,
            last_finalized_and_applied,
        }
    }

    /// Returns the last epoch finalized and applied so far.
    pub(crate) fn last_finalized_and_applied(&self) -> Option<EpochCommitment> {
        self.last_finalized_and_applied
    }

    /// Handles a new CSM client state: applies any newly finalized epochs and
    /// advances the internal progress marker.
    pub(crate) async fn handle_new_client_state(&mut self) -> CheckpointSyncResult<()> {
        let csm_status = self.ctx.fetch_csm_status().await?;
        debug!(?csm_status, "obtained csm status");
        let new_finalized = match (
            self.last_finalized_and_applied,
            csm_status.last_finalized_epoch,
        ) {
            (_, None) => {
                debug!("no finalized epoch in CSM status, skipping");
                return Ok(());
            }
            (None, Some(new_fin)) => {
                info!(%new_fin, "first finalized epoch observed");
                new_fin
            }
            (Some(prev), Some(new_fin)) => {
                if prev == new_fin {
                    debug!(%prev, "finalized epoch unchanged, skipping");
                    return Ok(());
                }
                debug!(%prev, %new_fin, "new finalized epoch");
                new_fin
            }
        };

        // Ensure the checkpoint is actually observed on L1 before catching up.
        let l1_ref = self
            .ctx
            .get_checkpoint_l1_ref(new_finalized)
            .await?
            .ok_or(CheckpointSyncError::MissingL1Ref(new_finalized))?;

        debug!(
            %new_finalized,
            l1_height = l1_ref.block_height(),
            "checking previous unapplied and applying new finalized checkpoint"
        );

        let last_applied =
            find_and_apply_unapplied_epochs(self.ctx.as_ref(), new_finalized).await?;

        self.last_finalized_and_applied = last_applied;
        info!(?last_applied, "checkpoint sync advanced");

        Ok(())
    }
}

/// Applies a single finalized epoch: reconstructs its state via the chain worker,
/// advances the safe tip, finalizes it, and publishes the resulting sync status.
///
/// All DA decoding, manifest fetching and state reconstruction happen inside the
/// chain worker.
pub(crate) async fn apply_and_finalize_epoch(
    ctx: &impl CheckpointSyncCtx,
    epoch: EpochCommitment,
) -> CheckpointSyncResult<()> {
    debug!(%epoch, "reconstructing epoch state via chain worker");
    ctx.apply_checkpoint(epoch).await?;

    finalize_and_publish(ctx, epoch).await?;

    info!(%epoch, "checkpoint applied and finalized");
    Ok(())
}

/// Re-runs the safe-tip + finalize + publish-status tail of [`apply_and_finalize_epoch`]
/// for an epoch whose state is already persisted.
///
/// Used at startup to recover from a crash that left the summary written but
/// finalization unfinished: idempotent calls, no re-application.
pub(crate) async fn refinalize_applied_epoch(
    ctx: &impl CheckpointSyncCtx,
    epoch: EpochCommitment,
) -> CheckpointSyncResult<()> {
    debug!(%epoch, "re-finalizing already-applied epoch");
    finalize_and_publish(ctx, epoch).await
}

/// Update safe tip, finalize epoch, build & publish sync status.
async fn finalize_and_publish(
    ctx: &impl CheckpointSyncCtx,
    epoch: EpochCommitment,
) -> CheckpointSyncResult<()> {
    debug!(%epoch, "updating safe tip");
    ctx.update_safe_tip(epoch.to_block_commitment()).await?;

    debug!(%epoch, "finalizing epoch");
    ctx.finalize_epoch(epoch).await?;

    let status = build_ol_sync_status(ctx, epoch).await?;
    ctx.publish_ol_sync_status(status);
    Ok(())
}

/// Builds an [`OLSyncStatus`] from a finalized epoch's summary.
pub(crate) async fn build_ol_sync_status(
    ctx: &impl CheckpointSyncCtx,
    epoch: EpochCommitment,
) -> CheckpointSyncResult<OLSyncStatus> {
    let summary = ctx
        .get_epoch_summary(epoch)
        .await?
        .ok_or(CheckpointSyncError::MissingEpochSummary(epoch))?;
    let terminal = *summary.terminal();
    let epoch_num = summary.epoch();
    let new_l1 = *summary.new_l1();
    // Epoch 0 has no predecessor; `null` is the canonical genesis-prev value.
    let prev_epoch = summary
        .get_prev_epoch_commitment()
        .unwrap_or(EpochCommitment::null());

    // Checkpoint sync always lands on terminal blocks, and for it
    // confirmed == finalized (5th and 6th args).
    Ok(OLSyncStatus::new(
        terminal, epoch_num, true, prev_epoch, epoch, epoch, new_l1,
    ))
}

/// Scans for unapplied finalized epochs and applies them in chronological order.
///
/// Returns the last applied epoch, or `None` if there is nothing to apply.
pub(crate) async fn find_and_apply_unapplied_epochs(
    ctx: &impl CheckpointSyncCtx,
    cur_finalized: EpochCommitment,
) -> CheckpointSyncResult<Option<EpochCommitment>> {
    let l1_tip_height = ctx
        .fetch_l1_tip_height()
        .await?
        .ok_or(CheckpointSyncError::L1TipNotReady)?;
    let reorg_safe_depth = ctx.rollup_params().l1_reorg_safe_depth;
    debug!(
        %cur_finalized,
        l1_tip_height,
        reorg_safe_depth,
        "scanning for unapplied finalized epochs"
    );

    let (mut last_applied_epoch, unapplied_epochs) =
        scan_unapplied_epochs(ctx, cur_finalized, l1_tip_height, reorg_safe_depth).await?;

    let num_unapplied = unapplied_epochs.len();
    if num_unapplied > 0 {
        info!(
            num_unapplied,
            ?last_applied_epoch,
            "catching up on unapplied epochs"
        );
    } else {
        debug!(?last_applied_epoch, "all epochs already applied");
    }

    // Apply oldest-first (scan collects newest-first).
    for (i, epoch) in unapplied_epochs.into_iter().rev().enumerate() {
        info!(
            %epoch,
            progress = i + 1,
            total = num_unapplied,
            "applying epoch during init"
        );
        apply_and_finalize_epoch(ctx, epoch).await?;
        last_applied_epoch = Some(epoch);
    }
    Ok(last_applied_epoch)
}

/// Walks backwards from `start_finalized`, collecting reorg-safe epochs that have
/// not yet been applied. Stops at genesis or the first already-applied epoch.
///
/// Returns the last applied epoch (if any) and the unapplied epochs newest-first.
pub(crate) async fn scan_unapplied_epochs(
    ctx: &impl CheckpointSyncCtx,
    start_finalized: EpochCommitment,
    l1_tip_height: u32,
    reorg_safe_depth: u32,
) -> CheckpointSyncResult<(Option<EpochCommitment>, Vec<EpochCommitment>)> {
    let mut unapplied = Vec::new();
    let mut cur_finalized = start_finalized;

    let last_applied = loop {
        // Genesis is treated as already applied.
        if cur_finalized.epoch() == 0 {
            break Some(cur_finalized);
        }

        let l1_ref = ctx
            .get_checkpoint_l1_ref(cur_finalized)
            .await?
            .ok_or(CheckpointSyncError::MissingL1Ref(cur_finalized))?;

        let depth = l1_confirmations(l1_ref.block_height(), l1_tip_height);
        debug!(
            ?reorg_safe_depth,
            ?depth,
            ?l1_ref,
            ?cur_finalized,
            "l1 ref for checkpoint"
        );

        if !is_l1_reorg_safe(l1_ref.block_height(), l1_tip_height, reorg_safe_depth) {
            return Err(CheckpointSyncError::NotReorgSafe {
                epoch: cur_finalized,
                depth,
                required: reorg_safe_depth,
            });
        }

        // An epoch is applied iff its summary exists: the chain worker inserts
        // the summary after reconstructing the state.
        if ctx.get_epoch_summary(cur_finalized).await?.is_some() {
            debug!(%cur_finalized, "found already-applied epoch, stopping scan");
            break Some(cur_finalized);
        }
        debug!(%cur_finalized, "epoch not yet applied, queuing for catchup");
        unapplied.push(cur_finalized);
        // Periodic progress so a large catch-up scan is not invisible at info level.
        if unapplied.len() % 50 == 0 {
            info!(scanned = unapplied.len(), %cur_finalized, "scan in progress");
        }

        let prev_epoch_num = cur_finalized.epoch().saturating_sub(1);
        cur_finalized = ctx
            .get_canonical_epoch_commitment(prev_epoch_num)
            .await?
            .ok_or(CheckpointSyncError::MissingPredecessor(prev_epoch_num))?;
    };

    Ok((last_applied, unapplied))
}

impl<C> ServiceState for CheckpointSyncState<C>
where
    C: CheckpointSyncCtx + 'static,
{
    fn name(&self) -> &str {
        "checkpoint-sync"
    }
}
