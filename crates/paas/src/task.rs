//! Task types and lifecycle management
//!
//! This module defines the core task types used by the prover service:
//! - [`TaskId`]: Universal task identifier containing program and backend
//! - [`TaskStatus`]: Task lifecycle states (pending, queued, proving, completed, failed)

use serde::{Deserialize, Serialize};

use crate::{program::ProgramType, ZkVmBackend};

// ================================================================================================
// Task Identifier
// ================================================================================================

/// Universal task identifier with program and backend
///
/// TaskId is the internal identifier for tasks in the prover service. Users don't interact
/// with TaskId directly - they receive UUIDs from submit_task() and use those for tracking.
///
/// ## Example
///
/// ```rust,ignore
/// // Internal use only - not exposed to users
/// let task_id = TaskId::new(ProofTask(checkpoint), ZkVmBackend::SP1);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(bound = "P: ProgramType")]
pub struct TaskId<P: ProgramType> {
    /// The program to prove
    pub(crate) program: P,

    /// Backend to use for proving
    pub(crate) backend: ZkVmBackend,
}

impl<P: ProgramType> TaskId<P> {
    /// Create a new task ID
    pub fn new(program: P, backend: ZkVmBackend) -> Self {
        Self { program, backend }
    }

    /// Get a reference to the program
    pub fn program(&self) -> &P {
        &self.program
    }

    /// Get a reference to the backend
    pub fn backend(&self) -> &ZkVmBackend {
        &self.backend
    }
}

// ================================================================================================
// Task Status
// ================================================================================================

/// Task lifecycle status
///
/// Represents the current state of a proof generation task. Tasks progress through
/// several states from submission to completion or failure.
///
/// ## State Transitions
///
/// ```text
/// Pending → Queued → Proving → Completed
///                            ↘
///                              TransientFailure → (retry) → Queued
///                            ↘                   ↘
///                              PermanentFailure    → (max retries) → PermanentFailure
/// ```
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    borsh::BorshSerialize,
    borsh::BorshDeserialize,
)]
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

// ================================================================================================
// Task Result
// ================================================================================================

/// Result of awaitable task execution
///
/// Returned by `execute_task()` when waiting for a task to complete.
/// This is different from TaskStatus - it only represents final outcomes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskResult {
    /// Task completed successfully
    Completed { uuid: String },

    /// Task failed permanently (will not retry)
    Failed { uuid: String, error: String },
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    enum TestProgram {
        Program1,
        Program2,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    enum TestVariant {
        A,
        B,
    }

    impl ProgramType for TestProgram {
        type RoutingKey = TestVariant;

        fn routing_key(&self) -> Self::RoutingKey {
            match self {
                TestProgram::Program1 => TestVariant::A,
                TestProgram::Program2 => TestVariant::B,
            }
        }
    }

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

    #[test]
    fn test_task_status_is_retriable() {
        // Only TransientFailure should be retriable
        assert!(TaskStatus::TransientFailure {
            retry_count: 1,
            error: "test".into()
        }
        .is_retriable());

        // Other states are not retriable
        assert!(!TaskStatus::Pending.is_retriable());
        assert!(!TaskStatus::Queued.is_retriable());
        assert!(!TaskStatus::Proving.is_retriable());
        assert!(!TaskStatus::Completed.is_retriable());
        assert!(!TaskStatus::PermanentFailure {
            error: "test".into()
        }
        .is_retriable());
    }

    #[test]
    fn test_task_status_display() {
        let status = TaskStatus::Pending;
        let debug = format!("{:?}", status);
        assert!(debug.contains("Pending"));

        let status = TaskStatus::TransientFailure {
            retry_count: 2,
            error: "timeout".into(),
        };
        let debug = format!("{:?}", status);
        assert!(debug.contains("TransientFailure"));
        assert!(debug.contains("2"));
        assert!(debug.contains("timeout"));
    }

    #[test]
    fn test_task_id_equality() {
        let task1 = TaskId::new(TestProgram::Program1, ZkVmBackend::Native);
        let task2 = TaskId::new(TestProgram::Program1, ZkVmBackend::Native);
        let task3 = TaskId::new(TestProgram::Program2, ZkVmBackend::Native);
        let task4 = TaskId::new(TestProgram::Program1, ZkVmBackend::SP1);

        // Same program and backend should be equal
        assert_eq!(task1, task2);
        assert_eq!(task1.clone(), task2.clone());

        // Different program should not be equal
        assert_ne!(task1, task3);

        // Different backend should not be equal
        assert_ne!(task1, task4);
    }

    #[test]
    fn test_task_id_hash() {
        let mut set = HashSet::new();

        let task1 = TaskId::new(TestProgram::Program1, ZkVmBackend::Native);
        let task2 = TaskId::new(TestProgram::Program1, ZkVmBackend::Native);

        // Should only store one copy
        set.insert(task1);
        set.insert(task2);
        assert_eq!(set.len(), 1);

        // Different task should be added
        set.insert(TaskId::new(TestProgram::Program2, ZkVmBackend::Native));
        assert_eq!(set.len(), 2);
    }

    #[test]
    fn test_task_id_accessors() {
        let task = TaskId::new(TestProgram::Program1, ZkVmBackend::SP1);
        assert_eq!(*task.program(), TestProgram::Program1);
        assert_eq!(*task.backend(), ZkVmBackend::SP1);
    }

    #[test]
    fn test_task_id_serialization() {
        let task = TaskId::new(TestProgram::Program1, ZkVmBackend::Native);
        let json = serde_json::to_string(&task).unwrap();
        let deserialized: TaskId<TestProgram> = serde_json::from_str(&json).unwrap();
        assert_eq!(task, deserialized);
    }
}
