//! Command handle for interacting with prover service

use strata_db::traits::ProofDatabase;
use strata_primitives::proof::ProofContext;
use strata_service::CommandHandle;
use tokio::sync::watch;

use crate::{
    PaaSCommand, PaaSError, PaaSReport, PaaSStatus, TaskId, TaskStatus, commands::ProofData,
};

/// Handle for interacting with the prover service
///
/// Provides async methods for submitting proof tasks and querying their status.
/// This handle wraps the command channel and provides a type-safe API.
#[derive(Debug)]
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
    /// Note: Caller must ensure all dependencies are completed before creating this task.
    /// PaaS does not manage dependencies - tasks are created independently.
    ///
    /// Returns a TaskId that can be used to query status or retrieve the proof.
    pub async fn create_task(&self, context: ProofContext) -> Result<TaskId, PaaSError> {
        self.command_handle
            .send_and_wait(|completion| PaaSCommand::CreateTask {
                context,
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

    /// Lists all tasks with optional status filter
    pub async fn list_tasks(
        &self,
        filter: Option<crate::commands::TaskStatusFilter>,
    ) -> Result<Vec<(TaskId, TaskStatus)>, PaaSError> {
        self.command_handle
            .send_and_wait(|completion| PaaSCommand::ListTasks {
                status_filter: filter,
                completion,
            })
            .await
            .map_err(convert_service_error)?
    }

    /// Gets the ProofKey for a task
    pub async fn get_proof_key(
        &self,
        task_id: TaskId,
    ) -> Result<strata_primitives::proof::ProofKey, PaaSError> {
        self.command_handle
            .send_and_wait(|completion| PaaSCommand::GetProofKey {
                task_id,
                completion,
            })
            .await
            .map_err(convert_service_error)?
    }

    /// Lists pending tasks (ready to start)
    pub async fn list_pending_tasks(&self) -> Result<Vec<TaskId>, PaaSError> {
        use crate::commands::TaskStatusFilter;
        let tasks = self.list_tasks(Some(TaskStatusFilter::Pending)).await?;
        Ok(tasks.into_iter().map(|(id, _)| id).collect())
    }

    /// Lists retriable tasks (transient failures)
    pub async fn list_retriable_tasks(&self) -> Result<Vec<TaskId>, PaaSError> {
        use crate::commands::TaskStatusFilter;
        let tasks = self
            .list_tasks(Some(TaskStatusFilter::TransientFailure))
            .await?;
        Ok(tasks.into_iter().map(|(id, _)| id).collect())
    }

    /// Lists active tasks (queued or proving)
    pub async fn list_active_tasks(&self) -> Result<Vec<TaskId>, PaaSError> {
        use crate::commands::TaskStatusFilter;
        let tasks = self.list_tasks(Some(TaskStatusFilter::Active)).await?;
        Ok(tasks.into_iter().map(|(id, _)| id).collect())
    }

    /// Updates task status (for advanced use cases)
    ///
    /// Most users should rely on automatic status management by the worker pool.
    /// This method is provided for advanced scenarios where external systems need
    /// to update task status.
    pub async fn set_task_status(
        &self,
        task_id: TaskId,
        new_status: crate::TaskStatusUpdate,
    ) -> Result<(), PaaSError> {
        self.command_handle
            .send_and_wait(|completion| PaaSCommand::SetTaskStatus {
                task_id,
                new_status: new_status.clone(),
                completion,
            })
            .await
            .map_err(convert_service_error)?
    }

    // Internal methods used by WorkerPool
    // These are not part of the public API and should only be used by the worker pool

    /// Marks a task as queued (ready to prove)
    ///
    /// Internal method used by WorkerPool. Not part of public API.
    pub(crate) async fn mark_queued(&self, task_id: TaskId) -> Result<(), PaaSError> {
        use crate::TaskStatusUpdate;
        self.set_task_status(task_id, TaskStatusUpdate::Queued)
            .await
    }

    /// Marks a task as proving/in-progress
    ///
    /// Internal method used by WorkerPool. Not part of public API.
    pub(crate) async fn mark_proving(&self, task_id: TaskId) -> Result<(), PaaSError> {
        use crate::TaskStatusUpdate;
        self.set_task_status(task_id, TaskStatusUpdate::Proving)
            .await
    }

    /// Marks a task as completed
    ///
    /// Internal method used by WorkerPool. Not part of public API.
    pub(crate) async fn mark_completed(&self, task_id: TaskId) -> Result<(), PaaSError> {
        use crate::TaskStatusUpdate;
        self.set_task_status(task_id, TaskStatusUpdate::Completed)
            .await
    }

    /// Marks a task as having a transient failure (will retry)
    ///
    /// Internal method used by WorkerPool. Not part of public API.
    pub(crate) async fn mark_transient_failure(
        &self,
        task_id: TaskId,
        error: String,
    ) -> Result<(), PaaSError> {
        use crate::TaskStatusUpdate;
        self.set_task_status(task_id, TaskStatusUpdate::TransientFailure { error })
            .await
    }

    /// Marks a task as permanently failed
    ///
    /// Internal method used by WorkerPool. Not part of public API.
    pub(crate) async fn mark_failed(&self, task_id: TaskId, error: String) -> Result<(), PaaSError> {
        use crate::TaskStatusUpdate;
        self.set_task_status(task_id, TaskStatusUpdate::Failed { error })
            .await
    }
}

/// Helper to convert ServiceError to PaaSError
fn convert_service_error(err: strata_service::ServiceError) -> PaaSError {
    err.into()
}
