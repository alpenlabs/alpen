//! Handle for registry-based prover service

use std::sync::Arc;

use strata_service::{CommandHandle, ServiceMonitor};

use crate::{
    commands::ProverCommand,
    error::{ProverServiceError, ProverServiceResult},
    service::ProverServiceStatus,
    state::StatusSummary,
    task::TaskStatus,
    task_id::TaskId,
    ProgramType, ZkVmBackend,
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

    /// Submit a task for proving with clean API - no discriminants!
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// // Just pass your program and backend - PaaS handles routing
    /// handle.submit_task(MyProgram::VariantA(42), ZkVmBackend::SP1).await?;
    /// handle.submit_task(MyProgram::VariantB(start, end), ZkVmBackend::Native).await?;
    /// ```
    pub async fn submit_task(&self, program: P, backend: ZkVmBackend) -> ProverServiceResult<()> {
        let task_id = TaskId::new(program, backend);
        self.submit_task_id(task_id).await
    }

    /// Submit a task using a TaskId directly
    pub async fn submit_task_id(&self, task_id: TaskId<P>) -> ProverServiceResult<()> {
        let task_id_clone = task_id.clone();
        self.command_handle
            .send_and_wait(|completion| ProverCommand::SubmitTask {
                task_id: task_id_clone.clone(),
                completion,
            })
            .await
            .map_err(|e| ProverServiceError::Internal(e.into()))
    }

    /// Get task status
    pub async fn get_status(&self, task_id: &TaskId<P>) -> ProverServiceResult<TaskStatus> {
        self.command_handle
            .send_and_wait(|completion| ProverCommand::GetStatus {
                task_id: task_id.clone(),
                completion,
            })
            .await
            .map_err(|e| ProverServiceError::Internal(e.into()))
    }

    /// Get status summary
    pub async fn get_summary(&self) -> ProverServiceResult<StatusSummary> {
        self.command_handle
            .send_and_wait(|completion| ProverCommand::GetSummary { completion })
            .await
            .map_err(|e| ProverServiceError::Internal(e.into()))
    }

    /// Get the current service status summary
    pub fn get_current_status(&self) -> StatusSummary {
        self.monitor.get_current().summary
    }
}
