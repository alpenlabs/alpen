//! Prover service handle for API access

use std::sync::Arc;

use strata_service::{CommandHandle, ServiceMonitor};

use crate::commands::ProverCommand;
use crate::error::{PaaSError, PaaSResult};
use crate::service::ProverServiceStatus;
use crate::state::StatusSummary;
use crate::task::TaskStatus;
use crate::zkvm::{ProgramId, ZkVmTaskId};

/// Handle for interacting with the prover service
///
/// This handle provides a high-level API for submitting zkaleido tasks and querying status.
/// Uses the command handle pattern for async communication with the service.
///
/// Generic over `P: ProgramId` - your program identifier type.
#[derive(Clone)]
pub struct ProverHandle<P: ProgramId> {
    command_handle: Arc<CommandHandle<ProverCommand<ZkVmTaskId<P>>>>,
    monitor: ServiceMonitor<ProverServiceStatus>,
}

impl<P: ProgramId> ProverHandle<P> {
    /// Create a new handle
    pub fn new(
        command_handle: CommandHandle<ProverCommand<ZkVmTaskId<P>>>,
        monitor: ServiceMonitor<ProverServiceStatus>,
    ) -> Self {
        Self {
            command_handle: Arc::new(command_handle),
            monitor,
        }
    }

    /// Submit a task for proving
    pub async fn submit_task(&self, task_id: ZkVmTaskId<P>) -> PaaSResult<()> {
        let task_id_clone = task_id.clone();
        self.command_handle
            .send_and_wait(|completion| ProverCommand::SubmitTask {
                task_id: task_id_clone.clone(),
                completion
            })
            .await
            .map_err(|e| PaaSError::Internal(e.into()))
    }

    /// Get task status
    pub async fn get_status(&self, task_id: &ZkVmTaskId<P>) -> PaaSResult<TaskStatus> {
        self.command_handle
            .send_and_wait(|completion| ProverCommand::GetStatus {
                task_id: task_id.clone(),
                completion,
            })
            .await
            .map_err(|e| PaaSError::Internal(e.into()))
    }

    /// Get status summary
    pub async fn get_summary(&self) -> PaaSResult<StatusSummary> {
        self.command_handle
            .send_and_wait(|completion| ProverCommand::GetSummary { completion })
            .await
            .map_err(|e| PaaSError::Internal(e.into()))
    }

    /// Get the current service status summary
    pub fn get_current_status(&self) -> StatusSummary {
        self.monitor.get_current().summary
    }
}
