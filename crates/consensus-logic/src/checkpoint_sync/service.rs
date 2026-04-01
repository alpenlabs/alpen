use std::{marker::PhantomData, sync::Arc};

use serde::Serialize;
use strata_node_context::NodeContext;
use strata_ol_da::DAExtractor;
use strata_service::{AsyncService, Response, Service, ServiceBuilder, ServiceMonitor};

use crate::checkpoint_sync::{
    context::{CheckpointSyncCtx, CheckpointSyncCtxImpl},
    input::{CheckpointSyncEvent, CheckpointSyncInput},
    CheckpointSyncState,
};

#[derive(Clone, Debug)]
pub struct CheckpointSyncService<E: DAExtractor, C: CheckpointSyncCtx<E>> {
    _e: PhantomData<E>,
    _c: PhantomData<C>,
}

#[derive(Clone, Debug, Serialize)]
pub struct CheckpointSyncStatus;

impl<E, C> Service for CheckpointSyncService<E, C>
where
    E: DAExtractor + Send + Sync + 'static,
    C: CheckpointSyncCtx<E> + Send + Sync + 'static,
{
    type Msg = CheckpointSyncEvent;
    type State = CheckpointSyncState<E, C>;
    type Status = CheckpointSyncStatus;

    fn get_status(_s: &Self::State) -> Self::Status {
        CheckpointSyncStatus
    }
}

impl<E, C> AsyncService for CheckpointSyncService<E, C>
where
    E: DAExtractor + Send + Sync + 'static,
    C: CheckpointSyncCtx<E> + Send + Sync + 'static,
{
    async fn on_launch(_state: &mut Self::State) -> anyhow::Result<()> {
        Ok(())
    }

    async fn process_input(state: &mut Self::State, input: Self::Msg) -> anyhow::Result<Response> {
        match input {
            CheckpointSyncEvent::NewStateUpdate(st) => state.handle_new_client_state(&st).await?,
            CheckpointSyncEvent::Abort => return Ok(Response::ShouldExit),
        }
        Ok(Response::Continue)
    }
}

pub async fn start_css_service<E>(
    nodectx: &NodeContext,
    chain_worker: Arc<strata_chain_worker_new::ChainWorkerHandle>,
    da_extractor: E,
) -> anyhow::Result<ServiceMonitor<CheckpointSyncStatus>>
where
    E: DAExtractor + Clone + Send + Sync + 'static,
{
    let ctx = CheckpointSyncCtxImpl::new(nodectx.storage().clone(), chain_worker, da_extractor);
    let clstate_rx = nodectx.status_channel().subscribe_checkpoint_state();

    let state = CheckpointSyncState::new(ctx);
    let input = CheckpointSyncInput::new(clstate_rx);

    let service_monitor = ServiceBuilder::<
        CheckpointSyncService<E, CheckpointSyncCtxImpl<E>>,
        CheckpointSyncInput,
    >::new()
    .with_state(state)
    .with_input(input)
    .launch_async("checkpoint-sync", nodectx.executor().as_ref())
    .await?;

    Ok(service_monitor)
}
