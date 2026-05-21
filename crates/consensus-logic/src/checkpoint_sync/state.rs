//! Service state and checkpoint application logic for the checkpoint sync service.

use std::sync::Arc;

use anyhow::anyhow;
use strata_db_types::DbError;
use strata_primitives::EpochCommitment;
use strata_service::ServiceState;
use strata_status::OLSyncStatus;
use tracing::{debug, info};

use crate::checkpoint_sync::{
    context::CheckpointSyncCtx, service::find_and_apply_unapplied_epochs,
};

/// Service state for the checkpoint sync service.
#[derive(Debug, Clone)]
pub struct CheckpointSyncState<C: CheckpointSyncCtx> {
    /// Dependency context.
    ctx: Arc<C>,
    /// Mutable progress tracking.
    inner: InnerState,
}

/// Progress tracking for the checkpoint sync service.
#[derive(Clone, Debug)]
pub(crate) struct InnerState {
    /// Last epoch that has been both finalized and applied to OL state.
    last_finalized_and_applied: Option<EpochCommitment>,
}

impl InnerState {
    pub(crate) fn new(last_finalized_epoch: Option<EpochCommitment>) -> Self {
        Self {
            last_finalized_and_applied: last_finalized_epoch,
        }
    }

    pub(crate) fn last_finalized_epoch(&self) -> Option<EpochCommitment> {
        self.last_finalized_and_applied
    }
}

impl<C: CheckpointSyncCtx> CheckpointSyncState<C> {
    pub(crate) fn new(ctx: Arc<C>, inner: InnerState) -> Self {
        Self { ctx, inner }
    }

    /// Handles a new CSM client state: applies any newly finalized epochs and
    /// advances the internal progress marker.
    pub(crate) async fn handle_new_client_state(&mut self) -> anyhow::Result<()> {
        let csm_status = self.ctx.fetch_csm_status().await?;
        debug!(?csm_status, "obtained csm status");
        let new_finalized = match (
            self.inner.last_finalized_and_applied,
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
            .ok_or_else(|| {
                anyhow!("L1 reference not found for finalized epoch: {new_finalized}")
            })?;

        debug!(
            %new_finalized,
            l1_height = l1_ref.block_height(),
            "checking previous unapplied and applying new finalized checkpoint"
        );

        let last_applied =
            find_and_apply_unapplied_epochs(self.ctx.as_ref(), new_finalized).await?;

        self.inner.last_finalized_and_applied = last_applied;
        info!(?last_applied, "checkpoint sync advanced");

        Ok(())
    }
}

/// Applies a single finalized epoch: reconstructs its state via the chain worker,
/// advances the safe tip, finalizes it, and publishes the resulting sync status.
///
/// All DA decoding, manifest fetching and state reconstruction happen inside the
/// chain worker.
pub(crate) async fn apply_checkpoint(
    ctx: &impl CheckpointSyncCtx,
    epoch: EpochCommitment,
) -> anyhow::Result<()> {
    debug!(%epoch, "reconstructing epoch state via chain worker");
    ctx.apply_checkpoint(epoch).await?;

    let blk = epoch.to_block_commitment();
    debug!(%epoch, "updating safe tip");
    ctx.update_safe_tip(blk).await?;

    debug!(%epoch, "finalizing epoch");
    ctx.finalize_epoch(epoch).await?;

    debug!(%epoch, "building ol sync status after finalizing epoch");
    let status = build_ol_sync_status(ctx, epoch).await?;
    ctx.publish_ol_sync_status(status);

    info!(%epoch, "checkpoint applied and finalized");
    Ok(())
}

/// Builds an [`OLSyncStatus`] from a finalized epoch's summary.
pub(crate) async fn build_ol_sync_status(
    ctx: &impl CheckpointSyncCtx,
    epoch: EpochCommitment,
) -> anyhow::Result<OLSyncStatus> {
    let summary = ctx
        .get_epoch_summary(epoch)
        .await?
        .ok_or(DbError::NonExistentEntry)?;
    let terminal = *summary.terminal();
    let epoch_num = summary.epoch();
    let new_l1 = *summary.new_l1();
    let prev_epoch = summary
        .get_prev_epoch_commitment()
        .unwrap_or(EpochCommitment::null());

    // Checkpoint sync always lands on terminal blocks, and for it
    // confirmed == finalized (5th and 6th args).
    Ok(OLSyncStatus::new(
        terminal, epoch_num, true, prev_epoch, epoch, epoch, new_l1,
    ))
}

impl<C> ServiceState for CheckpointSyncState<C>
where
    C: CheckpointSyncCtx + 'static,
{
    fn name(&self) -> &str {
        "checkpoint-sync"
    }
}
