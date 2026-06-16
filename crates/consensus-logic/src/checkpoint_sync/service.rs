//! Service framework wiring for the checkpoint sync service.

use std::{marker::PhantomData, sync::Arc};

use serde::Serialize;
use strata_csm_types::CheckpointState;
use strata_primitives::EpochCommitment;
use strata_service::{
    AsyncService, Response, Service, ServiceBuilder, ServiceMonitor, TokioWatchInput,
};
use strata_tasks::TaskExecutor;
use tokio::sync::watch;
use tracing::{debug, info, warn};

use crate::checkpoint_sync::{
    context::CheckpointSyncCtx,
    errors::{CheckpointSyncError, CheckpointSyncResult},
    state::{
        build_ol_sync_status, find_and_apply_unapplied_epochs, refinalize_applied_epoch,
        CheckpointSyncState,
    },
};

/// Marker type implementing the [`Service`] trait for checkpoint sync.
#[derive(Clone, Debug)]
pub struct CheckpointSyncService<C: CheckpointSyncCtx> {
    /// Carries the context type parameter.
    _c: PhantomData<C>,
}

/// Status published by the checkpoint sync service.
#[derive(Clone, Debug, Default, Serialize)]
pub struct CheckpointSyncStatus {
    /// Epoch number of the last checkpoint applied and finalized, if any.
    pub last_finalized_and_applied_epoch: Option<u32>,
    /// Terminal slot of that epoch, if any.
    pub last_finalized_and_applied_slot: Option<u64>,
}

/// Handle type for the checkpoint sync service.
pub type CssServiceHandle = ServiceMonitor<CheckpointSyncStatus>;

impl<C> Service for CheckpointSyncService<C>
where
    C: CheckpointSyncCtx,
{
    type Msg = CheckpointState;
    type State = CheckpointSyncState<C>;
    type Status = CheckpointSyncStatus;

    fn get_status(s: &Self::State) -> Self::Status {
        let last = s.last_finalized_and_applied();
        CheckpointSyncStatus {
            last_finalized_and_applied_epoch: last.map(|e| e.epoch()),
            last_finalized_and_applied_slot: last.map(|e| e.last_slot()),
        }
    }
}

impl<C> AsyncService for CheckpointSyncService<C>
where
    C: CheckpointSyncCtx,
{
    async fn on_launch(_state: &mut Self::State) -> anyhow::Result<()> {
        Ok(())
    }

    async fn process_input(state: &mut Self::State, _input: Self::Msg) -> anyhow::Result<Response> {
        match state.handle_new_client_state().await {
            Ok(()) => {}
            // Wait condition, not a failure: the L1 tip will advance and the
            // next CSM update will re-trigger the scan.
            Err(CheckpointSyncError::NotReorgSafe {
                epoch,
                depth,
                required,
            }) => {
                warn!(
                    %epoch, depth, required,
                    "checkpoint not reorg-safe yet, will retry on next CSM update"
                );
            }
            // Pre-sync: btcio reader hasn't published an L1 tip yet.
            Err(CheckpointSyncError::L1TipNotReady) => {
                debug!("L1 tip not yet ready, will retry on next CSM update");
            }
            Err(e) => return Err(e.into()),
        }
        Ok(Response::Continue)
    }
}

/// Launches the checkpoint sync service and returns its monitor.
///
/// Takes the context and raw service inputs directly so this module needs no
/// dependency on `NodeContext`; the binary assembles `ctx`.
pub async fn start_css<C: CheckpointSyncCtx>(
    ctx: Arc<C>,
    checkpoint_state_rx: watch::Receiver<CheckpointState>,
    texec: Arc<TaskExecutor>,
) -> anyhow::Result<ServiceMonitor<CheckpointSyncStatus>> {
    info!("initializing checkpoint sync service");
    let last_finalized_and_applied = initialize_css_inner_state(ctx.as_ref()).await?;

    // Publish initial OL sync status so the RPC is populated from startup.
    match last_finalized_and_applied {
        Some(epoch) => {
            info!(%epoch, "resuming from last finalized epoch");
            let status = build_ol_sync_status(ctx.as_ref(), epoch).await?;
            ctx.publish_ol_sync_status(status);
        }
        None => {
            info!("no finalized epoch found, doing nothing");
        }
    }

    let state = CheckpointSyncState::new(ctx, last_finalized_and_applied);
    let input = TokioWatchInput::from_receiver(checkpoint_state_rx);

    let service_monitor =
        ServiceBuilder::<CheckpointSyncService<C>, TokioWatchInput<CheckpointState>>::new()
            .with_state(state)
            .with_input(input)
            .launch_async("checkpoint-sync", texec.as_ref())
            .await?;

    Ok(service_monitor)
}

/// Initializes css state by catching up on any unapplied finalized epochs at startup and returns
/// the resulting last-applied epoch.
///
/// Also re-runs finalization on the last already-applied epoch found by the
/// scan: if a previous run crashed between writing the summary and finalizing,
/// the chain worker's `last_finalized_epoch` would otherwise stay behind
/// silently. The re-finalize is idempotent.
async fn initialize_css_inner_state(
    ctx: &impl CheckpointSyncCtx,
) -> CheckpointSyncResult<Option<EpochCommitment>> {
    let Some(cur_finalized) = ctx.fetch_csm_status().await?.last_finalized_epoch else {
        debug!("no finalized checkpoint in client state, nothing to catch up on");
        return Ok(None);
    };

    let last_applied_epoch = match find_and_apply_unapplied_epochs(ctx, cur_finalized).await {
        Ok(v) => v,
        Err(CheckpointSyncError::NotReorgSafe {
            epoch,
            depth,
            required,
        }) => {
            debug!(
                %epoch, depth, required,
                "finalized checkpoint not reorg-safe at startup, deferring to next CSM update"
            );
            return Ok(None);
        }
        Err(CheckpointSyncError::L1TipNotReady) => {
            warn!("L1 tip not yet ready at startup, deferring to next CSM update");
            return Ok(None);
        }
        Err(e) => return Err(e),
    };
    if let Some(epoch) = last_applied_epoch {
        if epoch.epoch() > 0 {
            debug!(%epoch, "re-finalizing last applied epoch at startup");
            refinalize_applied_epoch(ctx, epoch).await?;
        }
    }
    Ok(last_applied_epoch)
}
