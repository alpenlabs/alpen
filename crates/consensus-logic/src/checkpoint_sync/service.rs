use std::{marker::PhantomData, sync::Arc};

use anyhow::anyhow;
use serde::Serialize;
use strata_chain_worker_new::ChainWorkerHandle;
use strata_csm_types::CheckpointL1Ref;
use strata_csm_worker::CsmWorkerStatus;
use strata_node_context::NodeContext;
use strata_ol_da::DAExtractor;
use strata_primitives::EpochCommitment;
use strata_service::{AsyncService, Response, Service, ServiceBuilder, ServiceMonitor};
use tracing::{debug, info, warn};

use crate::checkpoint_sync::{
    context::{CheckpointSyncCtx, CheckpointSyncCtxImpl},
    input::{CheckpointSyncEvent, CheckpointSyncInput},
    state::{apply_checkpoint, build_ol_sync_status, InnerState},
    CheckpointSyncState,
};

#[derive(Clone, Debug)]
pub struct CheckpointSyncService<C: CheckpointSyncCtx> {
    _c: PhantomData<C>,
}

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

pub async fn start_css_service<E>(
    nodectx: &NodeContext,
    chain_worker: Arc<ChainWorkerHandle>,
    csm_monitor: Arc<ServiceMonitor<CsmWorkerStatus>>,
    da_extractor: E,
) -> anyhow::Result<ServiceMonitor<CheckpointSyncStatus>>
where
    E: DAExtractor + Clone + Send + Sync + 'static,
{
    let ctx = CheckpointSyncCtxImpl::new(
        nodectx.storage().clone(),
        chain_worker,
        da_extractor,
        csm_monitor,
        nodectx.status_channel().clone(),
        nodectx.bitcoin_client().clone(),
        nodectx.params().rollup().clone(),
    );
    let clstate_rx = nodectx.status_channel().subscribe_checkpoint_state();

    info!("initializing checkpoint sync service");
    let inner_state = initialize_css_inner_state(&ctx).await?;

    // Publish initial OL sync status so the RPC is populated from startup.
    match inner_state.last_finalized_epoch() {
        Some(epoch) => {
            debug!(%epoch, "resuming from last finalized epoch");
            let status = build_ol_sync_status(&ctx, epoch).await?;
            ctx.publish_ol_sync_status(status);
        }
        None => {
            debug!("no finalized epoch found, doing nothing");
        }
    };

    let state = CheckpointSyncState::new(ctx, inner_state);
    let input = CheckpointSyncInput::new(clstate_rx);

    let service_monitor = ServiceBuilder::<
        CheckpointSyncService<CheckpointSyncCtxImpl<E>>,
        CheckpointSyncInput,
    >::new()
    .with_state(state)
    .with_input(input)
    .launch_async("checkpoint-sync", nodectx.executor().as_ref())
    .await?;

    Ok(service_monitor)
}

/// Traverses epochs backwards from the latest finalized checkpoint to find the last
/// applied epoch, then applies any subsequent reorg-safe epochs in chronological order.
async fn initialize_css_inner_state(ctx: &impl CheckpointSyncCtx) -> anyhow::Result<InnerState> {
    // Get finalized checkpoint, if none just exit because there's nothing to sync.
    let Some(cur_finalized) = ctx.fetch_csm_status().await?.last_finalized_epoch else {
        debug!("no finalized checkpoint in client state, nothing to catch up on");
        return Ok(InnerState::new(None));
    };

    let last_applied_epoch = find_and_apply_unapplied_epochs(ctx, cur_finalized).await?;

    Ok(InnerState::new(last_applied_epoch))
}

/// Searches for unapplied finalized epochs and returns last applied epoch. If none found, returns
/// `None`.
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

    // Apply the epochs in chronological order (oldest first).
    for (i, (l1ref, epoch)) in unapplied_epochs.into_iter().rev().enumerate() {
        info!(
            %epoch,
            progress = i + 1,
            total = num_unapplied,
            "applying epoch during init"
        );
        apply_checkpoint(ctx, epoch, l1ref).await?;
        last_applied_epoch = Some(epoch);
    }
    Ok(last_applied_epoch)
}

/// Walks backwards from `start_epoch`, collecting reorg-safe epochs that have not yet
/// been applied to the OL state. Stops at genesis or the first already-applied epoch.
///
/// Returns the last applied epoch (if any) and the unapplied epochs in newest-first order.
pub(crate) async fn scan_unapplied_epochs(
    ctx: &impl CheckpointSyncCtx,
    start_finalized: EpochCommitment,
    l1_tip_height: u32,
    reorg_safe_depth: u32,
) -> anyhow::Result<(
    Option<EpochCommitment>,
    Vec<(CheckpointL1Ref, EpochCommitment)>,
)> {
    let mut unapplied = Vec::new();
    let mut cur_finalized = start_finalized;

    let last_applied = loop {
        // Don't need to go beyond genesis epoch
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

        let is_finalized = depth >= reorg_safe_depth;
        if !is_finalized {
            return Err(anyhow!(
                "Obtained unfinalized epoch when the descendants are finalized: {}",
                cur_finalized
            ));
        }

        // Check if it has been applied. It is applied if the epoch summary is present because
        // chain worker inserts an epoch summary after executing the DA.
        if ctx.get_epoch_summary(cur_finalized).await?.is_some() {
            debug!(%cur_finalized, "found already-applied epoch, stopping scan");
            break Some(cur_finalized);
        }
        debug!(%cur_finalized, "epoch not yet applied, queuing for catchup");
        unapplied.push((l1_ref, cur_finalized));

        let prev_epoch = ctx
            .get_canonical_epoch_commitment(cur_finalized.epoch().saturating_sub(1))
            .await?;

        cur_finalized = prev_epoch.ok_or_else(|| {
            anyhow!(
                "predecessor epoch {} not found in db for finalized epoch",
                cur_finalized.epoch().saturating_sub(1)
            )
        })?;
    };

    Ok((last_applied, unapplied))
}

fn fin_epoch_err(epoch: EpochCommitment, item: &str) -> anyhow::Error {
    anyhow!("Finalized epoch {epoch} does not have {item} in db")
}
