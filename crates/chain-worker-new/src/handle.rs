//! Handle for interacting with the chain worker service.

use strata_identifiers::OLBlockCommitment;
use strata_primitives::epoch::EpochCommitment;
use strata_service::{CommandHandle, ServiceError};

use crate::{WorkerError, WorkerResult, message::ChainWorkerMessage};

/// Handle for interacting with the chain worker service.
///
/// This provides an ergonomic API for sending commands to the worker
/// and waiting for results.
#[derive(Debug)]
pub struct ChainWorkerHandle {
    command_handle: CommandHandle<ChainWorkerMessage>,
}

impl ChainWorkerHandle {
    /// Create a new chain worker handle from a service command handle.
    pub fn new(command_handle: CommandHandle<ChainWorkerMessage>) -> Self {
        Self { command_handle }
    }

    /// Returns the number of pending inputs that have not been processed yet.
    pub fn pending(&self) -> usize {
        self.command_handle.pending()
    }

    /// Tries to execute a block, returns the result (async).
    pub async fn try_exec_block(&self, block: OLBlockCommitment) -> WorkerResult<()> {
        self.command_handle
            .send_and_wait(|completion| ChainWorkerMessage::TryExecBlock(block, completion))
            .await
            .map_err(convert_service_error)?
    }

    /// Tries to execute a block, returns the result (blocking).
    pub fn try_exec_block_blocking(&self, block: OLBlockCommitment) -> WorkerResult<()> {
        self.command_handle
            .send_and_wait_blocking(|completion| {
                ChainWorkerMessage::TryExecBlock(block, completion)
            })
            .map_err(convert_service_error)?
    }

    /// Finalize an epoch, making whatever database changes necessary (async).
    pub async fn finalize_epoch(&self, epoch: EpochCommitment) -> WorkerResult<()> {
        self.command_handle
            .send_and_wait(|completion| ChainWorkerMessage::FinalizeEpoch(epoch, completion))
            .await
            .map_err(convert_service_error)?
    }

    /// Finalize an epoch, making whatever database changes necessary (blocking).
    pub fn finalize_epoch_blocking(&self, epoch: EpochCommitment) -> WorkerResult<()> {
        self.command_handle
            .send_and_wait_blocking(|completion| {
                ChainWorkerMessage::FinalizeEpoch(epoch, completion)
            })
            .map_err(convert_service_error)?
    }

    /// Update the safe tip (async).
    pub async fn update_safe_tip(&self, safe_tip: OLBlockCommitment) -> WorkerResult<()> {
        self.command_handle
            .send_and_wait(|completion| ChainWorkerMessage::UpdateSafeTip(safe_tip, completion))
            .await
            .map_err(convert_service_error)?
    }

    /// Update the safe tip (blocking).
    pub fn update_safe_tip_blocking(&self, safe_tip: OLBlockCommitment) -> WorkerResult<()> {
        self.command_handle
            .send_and_wait_blocking(|completion| {
                ChainWorkerMessage::UpdateSafeTip(safe_tip, completion)
            })
            .map_err(convert_service_error)?
    }
}

/// Convert service framework errors to worker errors.
fn convert_service_error(err: ServiceError) -> WorkerError {
    match err {
        ServiceError::WorkerExited | ServiceError::WorkerExitedWithoutResponse => {
            WorkerError::WorkerExited
        }
        ServiceError::WaitCancelled => {
            WorkerError::Unexpected("operation was cancelled".to_string())
        }
        ServiceError::BlockingThreadPanic(msg) => {
            WorkerError::Unexpected(format!("blocking thread panicked: {}", msg))
        }
        ServiceError::UnknownInputErr => WorkerError::Unexpected("unknown input error".to_string()),
    }
}
