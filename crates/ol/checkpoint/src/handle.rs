//! Handle for interacting with the OL checkpoint service.

use std::sync::Arc;

use strata_service::{CommandHandle, ServiceError};

use crate::{
    errors::{OLCheckpointError, WorkerResult},
    message::OLCheckpointMessage,
};

/// Handle for interacting with the OL checkpoint service.
#[derive(Debug, Clone)]
pub struct OLCheckpointHandle {
    command_handle: Arc<CommandHandle<OLCheckpointMessage>>,
}

impl OLCheckpointHandle {
    pub fn new(command_handle: CommandHandle<OLCheckpointMessage>) -> Self {
        Self {
            command_handle: Arc::new(command_handle),
        }
    }

    /// Triggers a single polling cycle (async).
    pub async fn tick(&self) -> WorkerResult<()> {
        self.command_handle
            .send_and_wait(OLCheckpointMessage::Tick)
            .await
            .map_err(convert_service_error)?
    }

    /// Triggers a single polling cycle (blocking).
    pub fn tick_blocking(&self) -> WorkerResult<()> {
        self.command_handle
            .send_and_wait_blocking(OLCheckpointMessage::Tick)
            .map_err(convert_service_error)?
    }
}

fn convert_service_error(err: ServiceError) -> OLCheckpointError {
    match err {
        ServiceError::WorkerExited | ServiceError::WorkerExitedWithoutResponse => {
            OLCheckpointError::Unexpected("worker exited".to_string())
        }
        ServiceError::WaitCancelled => {
            OLCheckpointError::Unexpected("operation was cancelled".to_string())
        }
        ServiceError::BlockingThreadPanic(msg) => {
            OLCheckpointError::Unexpected(format!("blocking thread panicked: {}", msg))
        }
        ServiceError::UnknownInputErr => {
            OLCheckpointError::Unexpected("unknown input error".to_string())
        }
    }
}
