use anyhow::anyhow;
use strata_chain_worker_new::ApplyDAPayload;
use strata_csm_types::CheckpointL1Ref;
use strata_db_types::DbError;
use strata_ledger_types::IStateAccessor;
use strata_ol_chain_types_new::OLL1ManifestContainer;
use strata_ol_state_types::OLState;
use strata_primitives::EpochCommitment;
use strata_service::ServiceState;
use strata_status::OLSyncStatus;
use tracing::{debug, info};

use crate::checkpoint_sync::{
    context::CheckpointSyncCtx,
    service::{find_and_apply_unapplied_epochs, scan_unapplied_epochs},
};

#[derive(Debug, Clone)]
pub struct CheckpointSyncState<C: CheckpointSyncCtx> {
    ctx: C,
    inner: InnerState,
}

#[derive(Clone, Debug)]
pub(crate) struct InnerState {
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
    pub(crate) fn new(ctx: C, inner: InnerState) -> Self {
        Self { ctx, inner }
    }

    pub(crate) async fn handle_new_client_state(&mut self) -> Result<(), anyhow::Error> {
        let csm_status = self.ctx.fetch_csm_status().await?;
        debug!(?csm_status, "Obtained csm status");
        let new_finalized = csm_status.last_finalized_epoch;
        let new_finalized = match (self.inner.last_finalized_and_applied, new_finalized) {
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
                };
                debug!(%prev, %new_fin, "new finalized epoch");
                new_fin
            }
        };

        let l1_ref = self
            .ctx
            .fetch_l1_reference(new_finalized)
            .await?
            .ok_or_else(|| {
                anyhow!(
                    "L1 reference not found for finalized epoch: {}",
                    new_finalized
                )
            })?;

        debug!(
            %new_finalized,
            l1_height = l1_ref.block_height(),
            "checking previous unapplied and applying new finalized checkpoint"
        );

        let last_applied = find_and_apply_unapplied_epochs(&self.ctx, new_finalized).await?;

        // Update internal state
        self.inner.last_finalized_and_applied = last_applied;
        info!(?last_applied, "checkpoint sync advanced");

        Ok(())
    }
}

pub(crate) async fn apply_checkpoint(
    ctx: &impl CheckpointSyncCtx,
    epoch: EpochCommitment,
    l1ref: CheckpointL1Ref,
) -> anyhow::Result<()> {
    debug!(%epoch, "extracting DA and submitting to chain worker");
    extract_checkpoint_and_submit_to_chain_worker(epoch, l1ref, ctx).await?;

    let blk = epoch.to_block_commitment();

    debug!(%epoch, "updating safe tip");
    ctx.chain_worker().update_safe_tip(blk).await?;

    debug!(%epoch, "finalizing epoch");
    ctx.chain_worker().finalize_epoch(epoch).await?;

    debug!(%epoch, "building ol sync status after finalizing epoch");
    let status = build_ol_sync_status(ctx, epoch).await?;
    ctx.publish_ol_sync_status(status);

    info!(%epoch, "checkpoint applied and finalized");

    Ok(())
}

async fn extract_checkpoint_and_submit_to_chain_worker<C: CheckpointSyncCtx>(
    new_epoch: EpochCommitment,
    l1ref: CheckpointL1Ref,
    ctx: &C,
) -> anyhow::Result<()> {
    let prev_epoch_num = new_epoch.epoch().saturating_sub(1);
    let prev_epoch = ctx
        .get_canonical_epoch_commitment(prev_epoch_num)
        .await?
        .ok_or_else(|| anyhow!("Expected epoch not found in db: {}", prev_epoch_num))?;
    let prev_terminal = prev_epoch.to_block_commitment();

    let prev_state: OLState = ctx.get_state_at(prev_terminal).await?;

    let manifest_start = prev_state.last_l1_height().saturating_add(1);
    let manifest_end = l1ref.l1_commitment.height();
    debug!(
        %new_epoch,
        l1_range = %format!("{manifest_start}..={manifest_end}"),
        "fetching ASM manifests"
    );

    let manifests = ctx
        .fetch_asm_manifests_range(manifest_start, manifest_end)
        .await?;

    debug!(
        %new_epoch,
        num_manifests = manifests.len(),
        "fetched ASM manifests, extracting DA"
    );

    let container = OLL1ManifestContainer::new(manifests)?;

    let da = ctx.extract_da_data(&l1ref).await?;
    let (da_payload, terminal_complement) = da.into_parts();

    let payload = ApplyDAPayload::new(da_payload, container, new_epoch, terminal_complement);

    debug!(%new_epoch, "submitting DA payload to chain worker");
    ctx.chain_worker().apply_da(&payload).await?;

    Ok(())
}

/// Builds an [`OLSyncStatus`] from a finalized epoch.
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

    Ok(OLSyncStatus::new(
        terminal, epoch_num, true, // checkpoint sync always lands on terminal blocks
        prev_epoch, epoch, // confirmed = finalized for checkpoint sync
        epoch, new_l1,
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
