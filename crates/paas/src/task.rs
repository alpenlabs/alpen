//! Task types and lifecycle management

use serde::{Deserialize, Serialize};

/// Trait for task identifiers used internally by the framework
///
/// This trait defines the requirements for types that can be used as task identifiers
/// in the PaaS framework. Task identifiers must be unique, hashable, cloneable, and
/// serializable.
///
/// Note: This is an internal trait. Users typically work with the concrete `TaskId<P>`
/// struct exported at the crate root.
pub trait TaskIdentifier:
    Clone
    + Eq
    + std::hash::Hash
    + std::fmt::Debug
    + Send
    + Sync
    + Serialize
    + for<'de> Deserialize<'de>
    + 'static
{
}

// Blanket implementation for types that satisfy the requirements
impl<T> TaskIdentifier for T where
    T: Clone
        + Eq
        + std::hash::Hash
        + std::fmt::Debug
        + Send
        + Sync
        + Serialize
        + for<'de> Deserialize<'de>
        + 'static
{
}

/// Task lifecycle status
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskStatus {
    /// Task is waiting to be assigned to a worker
    Pending,

    /// Task has been assigned to a worker queue
    Queued,

    /// Task is currently being proven
    Proving,

    /// Task completed successfully
    Completed,

    /// Task failed with a transient error and will be retried
    TransientFailure {
        /// Number of retry attempts so far
        retry_count: u32,
        /// Error message
        error: String,
    },

    /// Task failed with a permanent error and will not be retried
    PermanentFailure {
        /// Error message
        error: String,
    },
}

impl TaskStatus {
    /// Check if the task is in a final state (completed or permanently failed)
    pub fn is_final(&self) -> bool {
        matches!(
            self,
            TaskStatus::Completed | TaskStatus::PermanentFailure { .. }
        )
    }

    /// Check if the task can be retried
    pub fn is_retriable(&self) -> bool {
        matches!(self, TaskStatus::TransientFailure { .. })
    }

    /// Check if the task is in progress
    pub fn is_in_progress(&self) -> bool {
        matches!(self, TaskStatus::Queued | TaskStatus::Proving)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_task_status_predicates() {
        assert!(TaskStatus::Completed.is_final());
        assert!(TaskStatus::PermanentFailure {
            error: "test".into()
        }
        .is_final());
        assert!(!TaskStatus::Pending.is_final());

        assert!(TaskStatus::TransientFailure {
            retry_count: 1,
            error: "test".into()
        }
        .is_retriable());
        assert!(!TaskStatus::Completed.is_retriable());

        assert!(TaskStatus::Queued.is_in_progress());
        assert!(TaskStatus::Proving.is_in_progress());
        assert!(!TaskStatus::Pending.is_in_progress());
    }
}
