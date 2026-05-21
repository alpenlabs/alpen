//! Service framework wiring for the checkpoint sync service.

use std::{marker::PhantomData, sync::Arc};

use anyhow::anyhow;
use serde::Serialize;
use strata_csm_types::CheckpointState;
use strata_primitives::EpochCommitment;
use strata_service::{AsyncService, Response, Service, ServiceBuilder, ServiceMonitor};
use strata_tasks::TaskExecutor;
use tokio::sync::watch;
use tracing::{debug, info, warn};

use crate::checkpoint_sync::{
    context::CheckpointSyncCtx,
    input::{CheckpointSyncEvent, CheckpointSyncInput},
    state::{apply_checkpoint, build_ol_sync_status, CheckpointSyncState, InnerState},
};

/// Marker type implementing the [`Service`] trait for checkpoint sync.
#[derive(Clone, Debug)]
pub struct CheckpointSyncService<C: CheckpointSyncCtx> {
    /// Carries the context type parameter.
    _c: PhantomData<C>,
}

/// Status published by the checkpoint sync service.
#[derive(Clone, Debug, Serialize)]
pub struct CheckpointSyncStatus;

/// Handle type for the checkpoint sync service.
pub type CssServiceHandle = ServiceMonitor<CheckpointSyncStatus>;

impl<C> Service for CheckpointSyncService<C>
where
    C: CheckpointSyncCtx + 'static,
{
    type Msg = CheckpointSyncEvent;
    type State = CheckpointSyncState<C>;
    type Status = CheckpointSyncStatus;

    fn get_status(_s: &Self::State) -> Self::Status {
        CheckpointSyncStatus
    }
}

impl<C> AsyncService for CheckpointSyncService<C>
where
    C: CheckpointSyncCtx + 'static,
{
    async fn on_launch(_state: &mut Self::State) -> anyhow::Result<()> {
        Ok(())
    }

    async fn process_input(state: &mut Self::State, input: Self::Msg) -> anyhow::Result<Response> {
        match input {
            CheckpointSyncEvent::NewCsmStateUpdate => state.handle_new_client_state().await?,
            CheckpointSyncEvent::Abort => {
                warn!("checkpoint sync received abort signal, shutting down");
                return Ok(Response::ShouldExit);
            }
        }
        Ok(Response::Continue)
    }
}

/// Launches the checkpoint sync service and returns its monitor.
///
/// Takes the context and raw service inputs directly so this module needs no
/// dependency on `NodeContext`; the binary assembles `ctx`.
pub async fn start_css_service<C: CheckpointSyncCtx + 'static>(
    ctx: Arc<C>,
    checkpoint_state_rx: watch::Receiver<CheckpointState>,
    texec: Arc<TaskExecutor>,
) -> anyhow::Result<ServiceMonitor<CheckpointSyncStatus>> {
    info!("initializing checkpoint sync service");
    let inner_state = initialize_css_inner_state(ctx.as_ref()).await?;

    // Publish initial OL sync status so the RPC is populated from startup.
    match inner_state.last_finalized_epoch() {
        Some(epoch) => {
            debug!(%epoch, "resuming from last finalized epoch");
            let status = build_ol_sync_status(ctx.as_ref(), epoch).await?;
            ctx.publish_ol_sync_status(status);
        }
        None => {
            debug!("no finalized epoch found, doing nothing");
        }
    }

    let state = CheckpointSyncState::new(ctx, inner_state);
    let input = CheckpointSyncInput::new(checkpoint_state_rx);

    let service_monitor = ServiceBuilder::<CheckpointSyncService<C>, CheckpointSyncInput>::new()
        .with_state(state)
        .with_input(input)
        .launch_async("checkpoint-sync", texec.as_ref())
        .await?;

    Ok(service_monitor)
}

/// Catches up on any unapplied finalized epochs at startup and returns the
/// resulting last-applied epoch.
async fn initialize_css_inner_state(ctx: &impl CheckpointSyncCtx) -> anyhow::Result<InnerState> {
    let Some(cur_finalized) = ctx.fetch_csm_status().await?.last_finalized_epoch else {
        debug!("no finalized checkpoint in client state, nothing to catch up on");
        return Ok(InnerState::new(None));
    };

    let last_applied_epoch = find_and_apply_unapplied_epochs(ctx, cur_finalized).await?;
    Ok(InnerState::new(last_applied_epoch))
}

/// Scans for unapplied finalized epochs and applies them in chronological order.
///
/// Returns the last applied epoch, or `None` if there is nothing to apply.
pub(crate) async fn find_and_apply_unapplied_epochs(
    ctx: &impl CheckpointSyncCtx,
    cur_finalized: EpochCommitment,
) -> anyhow::Result<Option<EpochCommitment>> {
    let l1_tip_height = ctx.fetch_l1_tip_height().await?;
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
        apply_checkpoint(ctx, epoch).await?;
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
) -> anyhow::Result<(Option<EpochCommitment>, Vec<EpochCommitment>)> {
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
            .ok_or_else(|| fin_epoch_err(cur_finalized, "l1 observation entry"))?;

        let depth = l1_tip_height.saturating_sub(l1_ref.block_height());
        debug!(
            ?reorg_safe_depth,
            ?depth,
            ?l1_ref,
            ?cur_finalized,
            "l1 ref for checkpoint"
        );

        if depth < reorg_safe_depth {
            return Err(anyhow!(
                "obtained unfinalized epoch when descendants are finalized: {cur_finalized}"
            ));
        }

        // An epoch is applied iff its summary exists: the chain worker inserts
        // the summary after reconstructing the state.
        if ctx.get_epoch_summary(cur_finalized).await?.is_some() {
            debug!(%cur_finalized, "found already-applied epoch, stopping scan");
            break Some(cur_finalized);
        }
        debug!(%cur_finalized, "epoch not yet applied, queuing for catchup");
        unapplied.push(cur_finalized);

        let prev_epoch_num = cur_finalized.epoch().saturating_sub(1);
        cur_finalized = ctx
            .get_canonical_epoch_commitment(prev_epoch_num)
            .await?
            .ok_or_else(|| {
                anyhow!("predecessor epoch {prev_epoch_num} not found in db for finalized epoch")
            })?;
    };

    Ok((last_applied, unapplied))
}

/// Builds an error for a finalized epoch missing an expected db entry.
fn fin_epoch_err(epoch: EpochCommitment, item: &str) -> anyhow::Error {
    anyhow!("finalized epoch {epoch} does not have {item} in db")
}
