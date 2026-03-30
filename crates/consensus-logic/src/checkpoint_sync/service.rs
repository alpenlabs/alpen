use serde::Serialize;
use strata_service::{AsyncService, Response, Service};

use crate::checkpoint_sync::{input::CheckpointSyncEvent, CheckpointSyncState};

#[derive(Clone, Debug)]
pub struct CheckpointSyncService {}

#[derive(Clone, Debug, Serialize)]
pub struct CheckpointSyncStatus;

impl Service for CheckpointSyncService {
    type Msg = CheckpointSyncEvent;
    type State = CheckpointSyncState;
    type Status = CheckpointSyncStatus;

    fn get_status(_s: &Self::State) -> Self::Status {
        CheckpointSyncStatus
    }
}

impl AsyncService for CheckpointSyncService {
    async fn on_launch(_state: &mut Self::State) -> anyhow::Result<()> {
        Ok(())
    }

    async fn process_input(state: &mut Self::State, input: &Self::Msg) -> anyhow::Result<Response> {
        match input {
            CheckpointSyncEvent::NewStateUpdate(st) => state.handle_new_client_state(st).await?,
            CheckpointSyncEvent::Abort => return Ok(Response::ShouldExit),
        }
        Ok(Response::Continue)
    }
}
