use std::marker::PhantomData;

use serde::Serialize;
use strata_ol_da::DAExtractor;
use strata_service::{AsyncService, Response, Service};

use crate::checkpoint_sync::{
    context::CheckpointSyncCtx, input::CheckpointSyncEvent, CheckpointSyncState,
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
