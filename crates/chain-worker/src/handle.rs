use std::sync::Arc;

use strata_primitives::prelude::*;
use strata_service::{CommandHandle, ServiceError};
use tokio::sync::Mutex;

use crate::{service::ChainWorkerService, WorkerContext, WorkerError, WorkerResult, message::ChainWorkerMessage};

/// Handle for interacting with the chain worker service.
#[derive(Debug)]
pub struct ChainWorkerHandle<W: WorkerContext + Send + Sync + 'static> {
    shared: Arc<Mutex<WorkerShared>>,
    command_handle: CommandHandle<ChainWorkerService<W>>,
}

impl<W: WorkerContext + Send + Sync + 'static> ChainWorkerHandle<W> {
    /// Create a new chain worker handle from shared state and a service command handle.
    pub fn new(shared: Arc<Mutex<WorkerShared>>, command_handle: CommandHandle<ChainWorkerService<W>>) -> Self {
        Self { shared, command_handle }
    }

    /// Returns the number of pending inputs that have not been processed yet.
    pub fn pending(&self) -> usize {
        self.command_handle.pending()
    }

    /// Tries to execute a block, returns the result.
    pub async fn try_exec_block(&self, block: L2BlockCommitment) -> WorkerResult<()> {
        self.command_handle
            .send_and_wait(|tx| ChainWorkerMessage::TryExecBlock(block, tokio::sync::Mutex::new(Some(tx))))
            .await
            .map_err(convert_service_error)?
    }

    /// Tries to execute a block, returns the result.
    pub fn try_exec_block_blocking(&self, block: L2BlockCommitment) -> WorkerResult<()> {
        self.command_handle
            .send_and_wait_blocking(|tx| ChainWorkerMessage::TryExecBlock(block, tokio::sync::Mutex::new(Some(tx))))
            .map_err(convert_service_error)?
    }

    /// Finalize an epoch, making whatever database changes necessary.
    pub async fn finalize_epoch(&self, epoch: EpochCommitment) -> WorkerResult<()> {
        self.command_handle
            .send_and_wait(|tx| ChainWorkerMessage::FinalizeEpoch(epoch, tokio::sync::Mutex::new(Some(tx))))
            .await
            .map_err(convert_service_error)?
    }

    /// Finalize an epoch, making whatever database changes necessary.
    pub fn finalize_epoch_blocking(&self, epoch: EpochCommitment) -> WorkerResult<()> {
        self.command_handle
            .send_and_wait_blocking(|tx| ChainWorkerMessage::FinalizeEpoch(epoch, tokio::sync::Mutex::new(Some(tx))))
            .map_err(convert_service_error)?
    }

    /// Update the safe tip, making whatever database changes necessary.
    pub async fn update_safe_tip(&self, safe_tip: L2BlockCommitment) -> WorkerResult<()> {
        self.command_handle
            .send_and_wait(|tx| ChainWorkerMessage::UpdateSafeTip(safe_tip, tokio::sync::Mutex::new(Some(tx))))
            .await
            .map_err(convert_service_error)?
    }

    /// Update the safe tip, making whatever database changes necessary.
    pub fn update_safe_tip_blocking(&self, safe_tip: L2BlockCommitment) -> WorkerResult<()> {
        self.command_handle
            .send_and_wait_blocking(|tx| ChainWorkerMessage::UpdateSafeTip(safe_tip, tokio::sync::Mutex::new(Some(tx))))
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
        ServiceError::UnknownInputErr => {
            WorkerError::Unexpected("unknown input error".to_string())
        }
    }
}

/// Input to the worker, reading inputs from the worker handle.
#[derive(Debug)]
pub struct ChainWorkerInput {
    shared: Arc<Mutex<WorkerShared>>,
    msg_rx: tokio::sync::mpsc::Receiver<ChainWorkerMessage>,
}

impl ChainWorkerInput {
    pub fn new(
        shared: Arc<Mutex<WorkerShared>>,
        msg_rx: tokio::sync::mpsc::Receiver<ChainWorkerMessage>,
    ) -> Self {
        Self { shared, msg_rx }
    }

    pub fn shared(&self) -> &Mutex<WorkerShared> {
        &self.shared
    }

    pub(crate) fn recv_next(&mut self) -> Option<ChainWorkerMessage> {
        self.msg_rx.blocking_recv()
    }
}

/// Shared state between the worker and the handle.
#[derive(Debug, Clone, Default)]
pub struct WorkerShared {
    // TODO
}
