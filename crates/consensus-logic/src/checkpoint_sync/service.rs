use std::{marker::PhantomData, sync::Arc};

use anyhow::anyhow;
use bitcoind_async_client::traits::Reader;
use serde::Serialize;
use strata_chain_worker_new::ChainWorkerHandle;
use strata_csm_types::CheckpointL1Ref;
use strata_csm_worker::CsmWorkerStatus;
use strata_node_context::NodeContext;
use strata_ol_da::DAExtractor;
use strata_primitives::{Buf32, EpochCommitment, L1BlockCommitment};
use strata_service::{AsyncService, Response, Service, ServiceBuilder, ServiceMonitor};

use crate::checkpoint_sync::{
    context::{CheckpointSyncCtx, CheckpointSyncCtxImpl},
    input::{CheckpointSyncEvent, CheckpointSyncInput},
    state::{apply_checkpoint, build_ol_sync_status, genesis_ol_sync_status, InnerState},
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
            CheckpointSyncEvent::Abort => return Ok(Response::ShouldExit),
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
    );
    let clstate_rx = nodectx.status_channel().subscribe_checkpoint_state();

    let inner_state = initialize_css_inner_state(nodectx, &ctx).await?;

    // Publish initial OL sync status so the RPC is populated from startup.
    let initial_status = match inner_state.last_finalized_epoch() {
        Some(epoch) => build_ol_sync_status(&ctx, epoch).await?,
        None => genesis_ol_sync_status(),
    };
    ctx.publish_ol_sync_status(initial_status);

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
async fn initialize_css_inner_state(
    nodectx: &NodeContext,
    ctx: &impl CheckpointSyncCtx,
) -> anyhow::Result<InnerState> {
    // No finalized checkpoint yet — nothing to sync.
    let Some(last_finalized) = nodectx
        .status_channel()
        .get_cur_client_state()
        .get_last_finalized_checkpoint()
    else {
        return Ok(InnerState::new(None));
    };

    let cur_finalized = last_finalized.batch_info.get_epoch_commitment();
    let l1_tip_height = nodectx.bitcoin_client().get_blockchain_info().await?.blocks;
    let reorg_safe_depth = nodectx.params().rollup().l1_reorg_safe_depth;

    //  Get unapplied epochs.
    let (mut last_applied_epoch, unapplied_epochs) =
        scan_unapplied_epochs(nodectx, cur_finalized, l1_tip_height, reorg_safe_depth).await?;

    // Apply the epochs in reverse order.
    for (l1ref, epoch) in unapplied_epochs.into_iter().rev() {
        apply_checkpoint(ctx, epoch, l1ref).await?;
        last_applied_epoch = Some(epoch);
    }

    Ok(InnerState::new(last_applied_epoch))
}

/// Walks backwards from `start_epoch`, collecting reorg-safe epochs that have not yet
/// been applied to the OL state. Stops at genesis or the first already-applied epoch.
///
/// Returns the last applied epoch (if any) and the unapplied epochs in newest-first order.
async fn scan_unapplied_epochs(
    nodectx: &NodeContext,
    start_finalized: EpochCommitment,
    l1_tip_height: u32,
    reorg_safe_depth: u32,
) -> anyhow::Result<(
    Option<EpochCommitment>,
    Vec<(CheckpointL1Ref, EpochCommitment)>,
)> {
    let mut unapplied = Vec::new();
    let mut last_applied = None;
    let mut cur_finalized = start_finalized;

    let ckpt_db = nodectx.storage().ol_checkpoint();
    let state_db = nodectx.storage().ol_state();

    loop {
        let l1_ref = ckpt_db
            .get_checkpoint_l1_ref_async(cur_finalized)
            .await?
            .ok_or_else(|| fin_epoch_err(cur_finalized, "l1 observation entry"))?;

        let is_finalized = l1_tip_height.saturating_sub(l1_ref.block_height()) >= reorg_safe_depth;
        if !is_finalized {
            return Err(anyhow!(
                "Obtained unfinalized epoch when the descendants are finalized: {}",
                cur_finalized
            ));
        }

        // Check if it has been applied. It is applied if the state db contains state corresponding
        // to the terminal OL block of this finalized epoch.
        if state_db
            .get_toplevel_ol_state_async(cur_finalized.to_block_commitment())
            .await?
            .is_some()
        {
            last_applied = Some(cur_finalized);
            break;
        }
        unapplied.push((l1_ref, cur_finalized));

        let summary = ckpt_db
            .get_epoch_summary_async(cur_finalized)
            .await?
            .ok_or_else(|| fin_epoch_err(cur_finalized, "epoch summary"))?;

        let prev_epoch = summary.get_prev_epoch_commitment();

        cur_finalized = if let Some(e) = prev_epoch { e } else { break };
    }

    Ok((last_applied, unapplied))
}

fn fin_epoch_err(epoch: EpochCommitment, item: &str) -> anyhow::Error {
    anyhow!("Finalized epoch {epoch} does not have {item} in db")
}
