//! Persistence traits for task tracking

use std::time::Instant;

use crate::{
    error::ProverServiceResult,
    task::{TaskId, TaskStatus},
    ProgramType,
};

/// Task metadata for persistence
#[derive(Debug, Clone)]
pub struct TaskRecord<T>
where
    T: Clone + Eq + std::hash::Hash + std::fmt::Debug + Send + Sync + 'static,
{
    pub task_id: T,
    pub uuid: String,
    pub status: TaskStatus,
    pub created_at: Instant,
    pub updated_at: Instant,
}

/// Trait for persistent task storage
///
/// Implementations should be database-backed for production use.
/// This trait enables idempotent task submission and crash recovery.
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
