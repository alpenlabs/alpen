//! Persistence traits for task tracking

use std::time::Instant;

use crate::{
    error::ProverServiceResult,
    program::ProgramType,
    task::{TaskId, TaskStatus},
};

/// Task metadata for persistence
#[derive(Debug, Clone)]
pub struct TaskRecord<T>
where
    T: Clone + Eq + std::hash::Hash + std::fmt::Debug + Send + Sync + 'static,
{
    task_id: T,
    uuid: String,
    status: TaskStatus,
    created_at: Instant,
    updated_at: Instant,
}

impl<T> TaskRecord<T>
where
    T: Clone + Eq + std::hash::Hash + std::fmt::Debug + Send + Sync + 'static,
{
    /// Create a new task record
    pub fn new(task_id: T, uuid: String, status: TaskStatus) -> Self {
        let now = Instant::now();
        Self {
            task_id,
            uuid,
            status,
            created_at: now,
            updated_at: now,
        }
    }

    /// Get the task ID
    pub fn task_id(&self) -> &T {
        &self.task_id
    }

    /// Get the UUID
    pub fn uuid(&self) -> &str {
        &self.uuid
    }

    /// Get the status
    pub fn status(&self) -> &TaskStatus {
        &self.status
    }

    /// Get creation timestamp
    pub fn created_at(&self) -> Instant {
        self.created_at
    }

    /// Get last update timestamp
    pub fn updated_at(&self) -> Instant {
        self.updated_at
    }

    /// Update the status and timestamp
    pub fn update_status(&mut self, status: TaskStatus) {
        self.status = status;
        self.updated_at = Instant::now();
    }
}

/// Trait for persistent task storage
///
/// Implementations should be database-backed for production use.
/// This trait enables idempotent task submission and crash recovery.
///
/// TODO: Ideally, this trait should be called from blocking-safe context
/// so we don't block the executor. Analyze usage and fix it.
pub trait TaskStore<P: ProgramType>: Send + Sync + 'static {
    /// Get UUID for a task if it exists
    fn get_uuid(&self, task_id: &TaskId<P>) -> Option<String>;

    /// Get full task record
    fn get_task(&self, task_id: &TaskId<P>) -> Option<TaskRecord<TaskId<P>>>;

    /// Get task by UUID
    fn get_task_by_uuid(&self, uuid: &str) -> Option<TaskRecord<TaskId<P>>>;

    /// Store a new task (returns error if task_id already exists)
    fn insert_task(&self, record: TaskRecord<TaskId<P>>) -> ProverServiceResult<()>;

    /// Update task status (returns error if task doesn't exist)
    fn update_status(&self, task_id: &TaskId<P>, status: TaskStatus) -> ProverServiceResult<()>;

    /// List all tasks matching a filter
    ///
    /// The filter function is boxed to make this trait dyn-compatible
    fn list_tasks(
        &self,
        filter: Box<dyn Fn(&TaskStatus) -> bool + '_>,
    ) -> Vec<TaskRecord<TaskId<P>>>;

    /// Get count of all tasks
    fn count(&self) -> usize;
}
