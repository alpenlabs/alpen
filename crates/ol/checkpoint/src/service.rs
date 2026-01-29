//! Service framework integration for OL checkpoint builder.

use serde::Serialize;
use strata_service::{Response, Service, SyncService};

use crate::{message::OLCheckpointMessage, state::OLCheckpointServiceState};

/// OL checkpoint builder service implementation.
#[derive(Debug)]
pub struct OLCheckpointService;

impl Service for OLCheckpointService {
    type State = OLCheckpointServiceState;
    type Msg = OLCheckpointMessage;
    type Status = OLCheckpointStatus;

    fn get_status(state: &Self::State) -> Self::Status {
        OLCheckpointStatus {
            is_initialized: state.is_initialized(),
            last_processed_epoch: state.last_processed_epoch(),
        }
    }
}

impl SyncService for OLCheckpointService {
    fn on_launch(state: &mut Self::State) -> anyhow::Result<()> {
        state.initialize()?;
        Ok(())
    }

    fn process_input(state: &mut Self::State, input: &Self::Msg) -> anyhow::Result<Response> {
        match input {
            OLCheckpointMessage::Tick(completion) => {
                let res = state.tick();
                completion.send_blocking(res);
            }
        }
        Ok(Response::Continue)
    }
}

/// Status information for the OL checkpoint builder service.
#[derive(Clone, Debug, Serialize)]
pub struct OLCheckpointStatus {
    /// Whether the worker has been initialized.
    pub is_initialized: bool,

    /// Last epoch processed, if any.
    pub last_processed_epoch: Option<u32>,
}
