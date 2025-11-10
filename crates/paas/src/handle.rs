//! Prover service handle for API access

use crate::error::PaaSResult;
use crate::state::StatusSummary;
use crate::task::{TaskId, TaskStatus};

/// Handle for interacting with the prover service
///
/// This handle provides a high-level API for submitting tasks and querying status.
/// It will be implemented using the command handle pattern in the future.
#[derive(Clone)]
pub struct ProverHandle<T: TaskId> {
    _phantom: std::marker::PhantomData<T>,
}

impl<T: TaskId> ProverHandle<T> {
    /// Create a new handle (placeholder)
    pub fn new() -> Self {
        Self {
            _phantom: std::marker::PhantomData,
        }
    }

    /// Submit a task for proving
    pub async fn submit_task(&self, _task_id: T) -> PaaSResult<()> {
        // TODO: Implement via command handle
        todo!("Implement via command handle pattern")
    }

    /// Get task status
    pub async fn get_status(&self, _task_id: &T) -> PaaSResult<TaskStatus> {
        // TODO: Implement via command handle
        todo!("Implement via command handle pattern")
    }

    /// Get status summary
    pub async fn get_summary(&self) -> PaaSResult<StatusSummary> {
        // TODO: Implement via command handle
        todo!("Implement via command handle pattern")
    }
}
