//! Command types for ProverService

use strata_service::CommandCompletionSender;

use crate::state::StatusSummary;
use crate::task::{TaskId, TaskStatus};

/// Commands that can be sent to ProverService
#[derive(Debug)]
pub enum ProverCommand<T: TaskId> {
    /// Submit a new task for proving
    SubmitTask {
        task_id: T,
        completion: CommandCompletionSender<()>,
    },

    /// Get the status of a task
    GetStatus {
        task_id: T,
        completion: CommandCompletionSender<TaskStatus>,
    },

    /// Get a status summary
    GetSummary {
        completion: CommandCompletionSender<StatusSummary>,
    },
}
