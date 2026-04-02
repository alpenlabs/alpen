use anyhow::anyhow;
use strata_chain_worker_new::ApplyDAPayload;
use strata_csm_types::{CheckpointL1Ref, ClientState};
use strata_ledger_types::IStateAccessor;
use strata_ol_chain_types_new::OLL1ManifestContainer;
use strata_ol_state_types::OLState;
use strata_primitives::EpochCommitment;
use strata_service::ServiceState;
use strata_status::OLSyncStatus;

use crate::checkpoint_sync::context::CheckpointSyncCtx;

#[derive(Debug, Clone)]
pub struct CheckpointSyncState<C: CheckpointSyncCtx> {
    ctx: C,
    inner: InnerState,
}

#[derive(Clone, Debug)]
pub(crate) struct InnerState {
    last_finalized_epoch: Option<EpochCommitment>,
}

impl InnerState {
    pub(crate) fn new(last_finalized_epoch: Option<EpochCommitment>) -> Self {
        Self {
            last_finalized_epoch,
        }
    }

    pub(crate) fn last_finalized_epoch(&self) -> Option<EpochCommitment> {
        self.last_finalized_epoch
    }
}

impl<C: CheckpointSyncCtx> CheckpointSyncState<C> {
    pub(crate) fn new(ctx: C, inner: InnerState) -> Self {
        Self { ctx, inner }
    }

    pub(crate) async fn handle_new_client_state(
        &mut self,
        client_state: &ClientState,
    ) -> Result<(), anyhow::Error> {
        let new_finalized_ckpt = client_state.get_last_finalized_checkpoint();

        let new_ckpt = match (self.inner.last_finalized_epoch, new_finalized_ckpt) {
            (_, None) => return Ok(()), // if new is none, do nothing
            (None, Some(new_ckpt)) => new_ckpt,
            (Some(prev), Some(new_ckpt)) => {
                let new = new_ckpt.batch_info.get_epoch_commitment();
                if prev == new {
                    return Ok(());
                };
                // Check continuity and validity
                validate_new_finalized_epoch(prev, new, &self.ctx)?;
                new_ckpt
            }
        };

        let epoch = new_ckpt.batch_info.get_epoch_commitment();
        apply_checkpoint(&self.ctx, epoch, new_ckpt.l1_reference).await?;

        // Update internal state
        self.inner.last_finalized_epoch = Some(epoch);

        Ok(())
    }
}

pub(crate) async fn apply_checkpoint(
    ctx: &impl CheckpointSyncCtx,
    epoch: EpochCommitment,
    l1ref: CheckpointL1Ref,
) -> anyhow::Result<()> {
    // Extract checkpoint and send to chain worker for processing DA.
    extract_checkpoint_and_submit_to_chain_worker(epoch, l1ref, ctx).await?;

    let blk = epoch.to_block_commitment();

    // Now that DA application is successful, update safe tip
    ctx.chain_worker().update_safe_tip(blk).await?;

    // And then finalize
    ctx.chain_worker().finalize_epoch(epoch).await?;

    let status = build_ol_sync_status(ctx, epoch)?;
    ctx.publish_ol_sync_status(status);

    Ok(())
}

fn validate_new_finalized_epoch<C: CheckpointSyncCtx>(
    prev: EpochCommitment,
    new: EpochCommitment,
    ctx: &C,
) -> Result<(), anyhow::Error> {
    let prev_summary = ctx.get_epoch_summary(prev)?;
    let new_summary = ctx.get_epoch_summary(new)?;
    if new_summary.prev_terminal() != prev_summary.terminal() {
        return Err(anyhow!(
            "Received incompatible finalized checkpoint {}",
            new
        ));
    }
    // TODO: any other checks?
    Ok(())
}

async fn extract_checkpoint_and_submit_to_chain_worker<C: CheckpointSyncCtx>(
    new_epoch: EpochCommitment,
    l1ref: CheckpointL1Ref,
    ctx: &C,
) -> anyhow::Result<()> {
    let new_summary = ctx.get_epoch_summary(new_epoch)?;
    let prev_terminal = new_summary.prev_terminal();

    let prev_state: OLState = ctx.get_state_at(*prev_terminal)?;

    let manifests = ctx.fetch_asm_manifests_range(
        // TODO: figure out the inclusiveness, by looking at block assembly
        prev_state.last_l1_height().saturating_add(1),
        l1ref.l1_commitment.height(),
    )?;

    let container = OLL1ManifestContainer::new(manifests)?;

    let da = ctx.extract_da_data(&l1ref)?;
    let (da_payload, terminal_complement) = da.into_parts();

    let payload = ApplyDAPayload::new(da_payload, container, new_epoch, terminal_complement);

    ctx.chain_worker().apply_da(&payload).await?;

    Ok(())
}

/// Builds an [`OLSyncStatus`] from a finalized epoch.
pub(crate) fn build_ol_sync_status(
    ctx: &impl CheckpointSyncCtx,
    epoch: EpochCommitment,
) -> anyhow::Result<OLSyncStatus> {
    let summary = ctx.get_epoch_summary(epoch)?;
    let terminal = *summary.terminal();
    let epoch_num = summary.epoch();
    let new_l1 = *summary.new_l1();
    let prev_epoch = summary
        .get_prev_epoch_commitment()
        .unwrap_or(EpochCommitment::null());

    Ok(OLSyncStatus::new(
        terminal,
        epoch_num,
        true, // checkpoint sync always lands on terminal blocks
        prev_epoch,
        epoch, // confirmed = finalized for checkpoint sync
        epoch,
        new_l1,
    ))
}

/// Builds a default genesis [`OLSyncStatus`].
pub(crate) fn genesis_ol_sync_status() -> OLSyncStatus {
    let null_epoch = EpochCommitment::null();
    OLSyncStatus::new(
        null_epoch.to_block_commitment(),
        0,
        false,
        null_epoch,
        null_epoch,
        null_epoch,
        Default::default(),
    )
}

impl<C> ServiceState for CheckpointSyncState<C>
where
    C: CheckpointSyncCtx + Send + Sync + 'static,
{
    fn name(&self) -> &str {
        "checkpoint-sync"
    }
}
