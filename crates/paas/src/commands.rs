//! Command messages for PaaS service

use strata_primitives::proof::ProofContext;
use strata_service::CommandCompletionSender;
use uuid::Uuid;

use crate::{PaaSError, PaaSReport, TaskStatus};

/// Task identifier (UUID v4)
pub type TaskId = Uuid;

/// Command messages for interacting with PaaS service
#[derive(Debug)]
pub enum PaaSCommand {
    /// Create a new proof task
    CreateTask {
        /// Proof context to generate
        context: ProofContext,
        /// Dependencies that must complete first
        deps: Vec<ProofContext>,
        /// Completion channel for response
        completion: CommandCompletionSender<Result<TaskId, PaaSError>>,
    },

    /// Get the status of a proof task
    GetTaskStatus {
        /// Task identifier
        task_id: TaskId,
        /// Completion channel for response
        completion: CommandCompletionSender<Result<TaskStatus, PaaSError>>,
    },

    /// Get a completed proof
    GetProof {
        /// Task identifier
        task_id: TaskId,
        /// Completion channel for response
        completion: CommandCompletionSender<Result<Option<ProofData>, PaaSError>>,
    },

    /// Cancel a pending or in-progress task
    CancelTask {
        /// Task identifier
        task_id: TaskId,
        /// Completion channel for response
        completion: CommandCompletionSender<Result<(), PaaSError>>,
    },

    /// Get service metrics report
    GetReport {
        /// Completion channel for response
        completion: CommandCompletionSender<Result<PaaSReport, PaaSError>>,
    },

    /// List tasks by status filter
    ListTasks {
        /// Status filter (None = all tasks)
        status_filter: Option<TaskStatusFilter>,
        /// Completion channel for response
        completion: CommandCompletionSender<Result<Vec<(TaskId, TaskStatus)>, PaaSError>>,
    },

    /// Get proof key for a task
    GetProofKey {
        /// Task identifier
        task_id: TaskId,
        /// Completion channel for response
        completion: CommandCompletionSender<Result<strata_primitives::proof::ProofKey, PaaSError>>,
    },

    /// Mark task as queued (used by worker pool)
    MarkQueued {
        /// Task identifier
        task_id: TaskId,
        /// Completion channel for response
        completion: CommandCompletionSender<Result<(), PaaSError>>,
    },

    /// Mark task as proving/in-progress (used by worker pool)
    MarkProving {
        /// Task identifier
        task_id: TaskId,
        /// Completion channel for response
        completion: CommandCompletionSender<Result<(), PaaSError>>,
    },

    /// Mark task as completed (used by worker pool)
    MarkCompleted {
        /// Task identifier
        task_id: TaskId,
        /// Completion channel for response
        completion: CommandCompletionSender<Result<(), PaaSError>>,
    },

    /// Mark task as failed with transient error (used by worker pool)
    MarkTransientFailure {
        /// Task identifier
        task_id: TaskId,
        /// Error message
        error: String,
        /// Completion channel for response
        completion: CommandCompletionSender<Result<(), PaaSError>>,
    },

    /// Mark task as permanently failed (used by worker pool)
    MarkFailed {
        /// Task identifier
        task_id: TaskId,
        /// Error message
        error: String,
        /// Completion channel for response
        completion: CommandCompletionSender<Result<(), PaaSError>>,
    },
}

/// Filter for querying tasks
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskStatusFilter {
    /// Pending tasks (ready to start)
    Pending,
    /// Queued tasks
    Queued,
    /// Currently proving
    Proving,
    /// Completed tasks
    Completed,
    /// Failed tasks (permanent)
    Failed,
    /// Cancelled tasks
    Cancelled,
    /// Tasks with transient failures (will retry)
    TransientFailure,
    /// Active tasks (queued or proving)
    Active,
}

/// Proof data returned from GetProof command
#[derive(Debug, Clone)]
pub struct ProofData {
    /// The proof receipt
    pub receipt: Vec<u8>,
    /// Public values (optional)
    pub public_values: Option<Vec<u8>>,
    /// Verification key (optional)
    pub verification_key: Option<Vec<u8>>,
}
