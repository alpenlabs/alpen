//! Command types for ProverService

use strata_service::CommandCompletionSender;

use crate::task::{TaskResult, TaskStatus};

/// Commands that can be sent to ProverService
///
/// Generic over T which represents the TaskId type
#[derive(Debug)]
pub enum ProverCommand<T>
where
    T: Clone + Eq + std::hash::Hash + std::fmt::Debug + Send + Sync + 'static,
{
    /// Submit a new task for proving (returns UUID)
    SubmitTask {
        task_id: T,
        completion: CommandCompletionSender<String>,
    },

    /// Execute a task and wait for completion (returns TaskResult)
    /// This is the awaitable version - blocks until task completes
    ExecuteTask {
        task_id: T,
        completion: CommandCompletionSender<TaskResult>,
    },

    /// Get the status of a task by UUID
    GetStatusByUuid {
        uuid: String,
        completion: CommandCompletionSender<TaskStatus>,
    },

    /// Get the status of a task by TaskId (internal use only)
    GetStatusByTaskId {
        task_id: T,
        completion: CommandCompletionSender<TaskStatus>,
    },
}
