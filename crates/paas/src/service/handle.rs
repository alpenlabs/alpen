//! Handle for registry-based prover service

use std::sync::Arc;

use strata_service::{CommandHandle, ServiceMonitor};

use crate::{
    error::{ProverServiceError, ProverServiceResult},
    program::ProgramType,
    service::{commands::ProverCommand, runtime::ProverServiceStatus, state::StatusSummary},
    task::{TaskId, TaskResult, TaskStatus},
    ZkVmBackend,
};

/// Handle for interacting with the prover service
///
/// This handle provides a clean API for submitting tasks without needing
/// to specify discriminants - just pass your program and backend.
#[derive(Clone)]
pub struct ProverHandle<P: ProgramType> {
    command_handle: Arc<CommandHandle<ProverCommand<TaskId<P>>>>,
    monitor: ServiceMonitor<ProverServiceStatus>,
}

impl<P: ProgramType> std::fmt::Debug for ProverHandle<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProverHandle")
            .field("monitor", &self.monitor)
            .finish()
    }
}

impl<P: ProgramType> ProverHandle<P> {
    /// Create a new handle
    pub fn new(
        command_handle: CommandHandle<ProverCommand<TaskId<P>>>,
        monitor: ServiceMonitor<ProverServiceStatus>,
    ) -> Self {
        Self {
            command_handle: Arc::new(command_handle),
            monitor,
        }
    }

    /// Submit a task for proving - returns tracking UUID
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// // Submit and receive UUID for tracking
    /// let uuid = handle.submit_task(MyProgram::VariantA(42), ZkVmBackend::SP1).await?;
    ///
    /// // Use UUID to check status
    /// let status = handle.get_status(&uuid).await?;
    /// ```
    pub async fn submit_task(
        &self,
        program: P,
        backend: ZkVmBackend,
    ) -> ProverServiceResult<String> {
        let task_id = TaskId::new(program, backend);
        self.submit_task_id(task_id).await
    }

    /// Submit a task using a TaskId directly (private - internal use only)
    async fn submit_task_id(&self, task_id: TaskId<P>) -> ProverServiceResult<String> {
        let task_id_clone = task_id.clone();
        self.command_handle
            .send_and_wait(|completion| ProverCommand::SubmitTask {
                task_id: task_id_clone.clone(),
                completion,
            })
            .await
            .map_err(|e| ProverServiceError::Internal(e.into()))
    }

    /// Execute a task and wait for completion (AWAITABLE)
    ///
    /// This method blocks until the task completes and returns the result.
    /// Unlike `submit_task()`, this does NOT return immediately - it waits
    /// for the proof to be generated.
    ///
    /// Use this when you need to know the task completed before continuing.
    /// Use `submit_task()` for fire-and-forget submission with polling.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// // Execute and wait for completion
    /// let result = handle.execute_task(MyProgram::VariantA(42), ZkVmBackend::SP1).await?;
    ///
    /// match result {
    ///     TaskResult::Completed { uuid } => {
    ///         println!("Task {} completed successfully", uuid);
    ///     }
    ///     TaskResult::Failed { uuid, error } => {
    ///         eprintln!("Task {} failed: {}", uuid, error);
    ///     }
    /// }
    /// ```
    pub async fn execute_task(
        &self,
        program: P,
        backend: ZkVmBackend,
    ) -> ProverServiceResult<TaskResult> {
        let task_id = TaskId::new(program, backend);
        self.command_handle
            .send_and_wait(|completion| ProverCommand::ExecuteTask {
                task_id: task_id.clone(),
                completion,
            })
            .await
            .map_err(|e| ProverServiceError::Internal(e.into()))
    }

    /// Get task status by UUID
    pub async fn get_status(&self, uuid: &str) -> ProverServiceResult<TaskStatus> {
        self.command_handle
            .send_and_wait(|completion| ProverCommand::GetStatusByUuid {
                uuid: uuid.to_string(),
                completion,
            })
            .await
            .map_err(|e| ProverServiceError::Internal(e.into()))
    }

    /// Get task status by TaskId (internal use - for RPC server compatibility)
    ///
    /// This is used internally when we have a TaskId but not a UUID (e.g., RPC queries).
    /// Regular users should use get_status(&uuid) instead.
    #[doc(hidden)]
    pub async fn get_status_by_task_id(
        &self,
        task_id: &TaskId<P>,
    ) -> ProverServiceResult<TaskStatus> {
        self.command_handle
            .send_and_wait(|completion| ProverCommand::GetStatusByTaskId {
                task_id: task_id.clone(),
                completion,
            })
            .await
            .map_err(|e| ProverServiceError::Internal(e.into()))
    }

    /// Get the current service status summary
    pub fn get_current_status(&self) -> StatusSummary {
        self.monitor.get_current().summary
    }
}
