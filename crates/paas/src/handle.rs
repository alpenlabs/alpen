//! Command handle for interacting with prover service

use strata_db::traits::ProofDatabase;
use strata_primitives::proof::ProofContext;
use strata_service::CommandHandle;
use tokio::sync::watch;

use crate::{
    commands::ProofData, PaaSCommand, PaaSError, PaaSReport, PaaSStatus, TaskId, TaskStatus,
};

/// Handle for interacting with the prover service
///
/// Provides async methods for submitting proof tasks and querying their status.
/// This handle wraps the command channel and provides a type-safe API.
pub struct ProverHandle<D: ProofDatabase> {
    command_handle: CommandHandle<PaaSCommand>,
    status_rx: watch::Receiver<PaaSStatus>,
    _phantom: std::marker::PhantomData<D>,
}

impl<D: ProofDatabase> ProverHandle<D> {
    /// Creates a new handle
    pub(crate) fn new(
        command_handle: CommandHandle<PaaSCommand>,
        status_rx: watch::Receiver<PaaSStatus>,
    ) -> Self {
        Self {
            command_handle,
            status_rx,
            _phantom: std::marker::PhantomData,
        }
    }

    /// Creates a new proof task
    ///
    /// Returns a TaskId that can be used to query status or retrieve the proof.
    pub async fn create_task(
        &self,
        context: ProofContext,
        deps: Vec<ProofContext>,
    ) -> Result<TaskId, PaaSError> {
        self.command_handle
            .send_and_wait(|completion| PaaSCommand::CreateTask {
                context,
                deps: deps.clone(),
                completion,
            })
            .await
            .map_err(convert_service_error)?
    }

    /// Gets the status of a proof task
    pub async fn get_task_status(&self, task_id: TaskId) -> Result<TaskStatus, PaaSError> {
        self.command_handle
            .send_and_wait(|completion| PaaSCommand::GetTaskStatus {
                task_id,
                completion,
            })
            .await
            .map_err(convert_service_error)?
    }

    /// Gets the proof data for a completed task
    ///
    /// Returns None if the task is not yet completed or if the proof is not available.
    pub async fn get_proof(&self, task_id: TaskId) -> Result<Option<ProofData>, PaaSError> {
        self.command_handle
            .send_and_wait(|completion| PaaSCommand::GetProof {
                task_id,
                completion,
            })
            .await
            .map_err(convert_service_error)?
    }

    /// Cancels a pending or in-progress task
    pub async fn cancel_task(&self, task_id: TaskId) -> Result<(), PaaSError> {
        self.command_handle
            .send_and_wait(|completion| PaaSCommand::CancelTask {
                task_id,
                completion,
            })
            .await
            .map_err(convert_service_error)?
    }

    /// Gets a comprehensive service metrics report
    pub async fn get_report(&self) -> Result<PaaSReport, PaaSError> {
        self.command_handle
            .send_and_wait(|completion| PaaSCommand::GetReport { completion })
            .await
            .map_err(convert_service_error)?
    }

    /// Gets the current service status (non-blocking)
    pub fn status(&self) -> PaaSStatus {
        self.status_rx.borrow().clone()
    }

    /// Gets a receiver for status updates
    pub fn status_rx(&self) -> watch::Receiver<PaaSStatus> {
        self.status_rx.clone()
    }
}

/// Helper to convert ServiceError to PaaSError
fn convert_service_error(err: strata_service::ServiceError) -> PaaSError {
    err.into()
}
