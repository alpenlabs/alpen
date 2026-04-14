//! Task lifecycle types.

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};

/// Status of a proof task in the lifecycle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub enum TaskStatus {
    /// Task registered but not yet picked up for proving.
    Pending,
    /// Task queued for the prove strategy.
    Queued,
    /// Actively being proved.
    Proving,
    /// Proof completed successfully, receipt available.
    Completed,
    /// Temporary failure; will be retried after backoff.
    TransientFailure { retry_count: u32, error: String },
    /// Unrecoverable failure; task will not be retried.
    PermanentFailure { error: String },
}

impl TaskStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::PermanentFailure { .. })
    }

    pub fn is_retriable(&self) -> bool {
        matches!(self, Self::TransientFailure { .. })
    }

    pub fn is_in_progress(&self) -> bool {
        matches!(self, Self::Queued | Self::Proving)
    }

    /// True for any status that should be re-spawned on startup recovery:
    /// tasks that were submitted but never finished (Pending / Queued /
    /// Proving). Transient failures are handled separately by the retry
    /// scanner via [`Self::is_retriable`].
    pub fn is_unfinished(&self) -> bool {
        matches!(self, Self::Pending | Self::Queued | Self::Proving)
    }
}

/// Outcome of a completed (or failed) task. Returned by `execute` and `wait_for_tasks`.
#[derive(Debug, Clone)]
pub enum TaskResult<T> {
    Completed { task: T },
    Failed { task: T, error: String },
}

impl<T> TaskResult<T> {
    pub fn completed(task: T) -> Self {
        Self::Completed { task }
    }

    pub fn failed(task: T, error: impl Into<String>) -> Self {
        Self::Failed {
            task,
            error: error.into(),
        }
    }

    pub fn is_completed(&self) -> bool {
        matches!(self, Self::Completed { .. })
    }

    pub fn is_failed(&self) -> bool {
        matches!(self, Self::Failed { .. })
    }

    pub fn task(&self) -> &T {
        match self {
            Self::Completed { task } | Self::Failed { task, .. } => task,
        }
    }
}
