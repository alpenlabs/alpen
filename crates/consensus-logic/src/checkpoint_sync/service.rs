//! Service framework wiring for the checkpoint sync service.

use std::{marker::PhantomData, sync::Arc};

use serde::Serialize;
use strata_csm_types::CheckpointState;
use strata_service::{AsyncService, Response, Service, ServiceBuilder, ServiceMonitor};
use strata_tasks::TaskExecutor;
use tokio::sync::watch;
use tracing::{debug, info, warn};

use crate::checkpoint_sync::{
    context::CheckpointSyncCtx,
    errors::{CheckpointSyncError, CheckpointSyncResult},
    input::{CheckpointSyncEvent, CheckpointSyncInput},
    state::{
        build_ol_sync_status, find_and_apply_unapplied_epochs, refinalize_applied_epoch,
        CheckpointSyncState, InnerState,
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
    C: CheckpointSyncCtx + 'static,
{
    type Msg = CheckpointSyncEvent;
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
    C: CheckpointSyncCtx + 'static,
{
    async fn on_launch(_state: &mut Self::State) -> anyhow::Result<()> {
        Ok(())
    }

    async fn process_input(state: &mut Self::State, input: Self::Msg) -> anyhow::Result<Response> {
        match input {
            CheckpointSyncEvent::NewCsmStateUpdate => match state.handle_new_client_state().await {
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
            },
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
    match inner_state.last_finalized_and_applied() {
        Some(epoch) => {
            info!(%epoch, "resuming from last finalized epoch");
            let status = build_ol_sync_status(ctx.as_ref(), epoch).await?;
            ctx.publish_ol_sync_status(status);
        }
        None => {
            info!("no finalized epoch found, doing nothing");
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
///
/// Also re-runs finalization on the last already-applied epoch found by the
/// scan: if a previous run crashed between writing the summary and finalizing,
/// the chain worker's `last_finalized_epoch` would otherwise stay behind
/// silently. The re-finalize is idempotent.
async fn initialize_css_inner_state(
    ctx: &impl CheckpointSyncCtx,
) -> CheckpointSyncResult<InnerState> {
    let Some(cur_finalized) = ctx.fetch_csm_status().await?.last_finalized_epoch else {
        debug!("no finalized checkpoint in client state, nothing to catch up on");
        return Ok(InnerState::new(None));
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
            return Ok(InnerState::new(None));
        }
        Err(CheckpointSyncError::L1TipNotReady) => {
            warn!("L1 tip not yet ready at startup, deferring to next CSM update");
            return Ok(InnerState::new(None));
        }
        Err(e) => return Err(e),
    };
    if let Some(epoch) = last_applied_epoch {
        if epoch.epoch() > 0 {
            debug!(%epoch, "re-finalizing last applied epoch at startup");
            refinalize_applied_epoch(ctx, epoch).await?;
        }
    }
    Ok(InnerState::new(last_applied_epoch))
}
