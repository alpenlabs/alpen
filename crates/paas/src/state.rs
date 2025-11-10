//! Service state management

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde::Serialize;
use strata_service::ServiceState;

use crate::config::PaaSConfig;
use crate::error::{PaaSError, PaaSResult};
use crate::task::{TaskId, TaskStatus};
use crate::Prover;

/// Task metadata tracked by the service
#[derive(Debug, Clone)]
struct TaskInfo<T: TaskId> {
    task_id: T,
    status: TaskStatus,
    created_at: Instant,
    updated_at: Instant,
}

/// Service state for ProverService
pub struct ProverServiceState<P: Prover> {
    /// The prover implementation
    prover: Arc<P>,

    /// Configuration
    config: PaaSConfig<P::Backend>,

    /// Task tracker (thread-safe)
    tasks: Arc<Mutex<HashMap<P::TaskId, TaskInfo<P::TaskId>>>>,

    /// In-progress tasks per backend (for worker limits)
    in_progress: Arc<Mutex<HashMap<P::Backend, usize>>>,
}

impl<P: Prover> ProverServiceState<P> {
    /// Create new service state
    pub fn new(prover: Arc<P>, config: PaaSConfig<P::Backend>) -> Self {
        Self {
            prover,
            config,
            tasks: Arc::new(Mutex::new(HashMap::new())),
            in_progress: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Submit a new task
    pub fn submit_task(&self, task_id: P::TaskId) -> PaaSResult<()> {
        let mut tasks = self.tasks.lock().unwrap();

        if tasks.contains_key(&task_id) {
            return Err(PaaSError::Config("Task already exists".into()));
        }

        let now = Instant::now();
        tasks.insert(
            task_id.clone(),
            TaskInfo {
                task_id,
                status: TaskStatus::Pending,
                created_at: now,
                updated_at: now,
            },
        );

        Ok(())
    }

    /// Get task status
    pub fn get_status(&self, task_id: &P::TaskId) -> PaaSResult<TaskStatus> {
        let tasks = self.tasks.lock().unwrap();
        tasks
            .get(task_id)
            .map(|info| info.status.clone())
            .ok_or_else(|| PaaSError::TaskNotFound(format!("{:?}", task_id)))
    }

    /// Update task status
    pub fn update_status(&self, task_id: &P::TaskId, status: TaskStatus) -> PaaSResult<()> {
        let mut tasks = self.tasks.lock().unwrap();
        let task = tasks
            .get_mut(task_id)
            .ok_or_else(|| PaaSError::TaskNotFound(format!("{:?}", task_id)))?;

        task.status = status;
        task.updated_at = Instant::now();

        Ok(())
    }

    /// List tasks by status filter
    pub fn list_tasks<F>(&self, filter: F) -> Vec<P::TaskId>
    where
        F: Fn(&TaskStatus) -> bool,
    {
        let tasks = self.tasks.lock().unwrap();
        tasks
            .values()
            .filter(|task| filter(&task.status))
            .map(|task| task.task_id.clone())
            .collect()
    }

    /// List pending tasks
    pub fn list_pending(&self) -> Vec<P::TaskId> {
        self.list_tasks(|status| matches!(status, TaskStatus::Pending))
    }

    /// List retriable tasks
    pub fn list_retriable(&self) -> Vec<P::TaskId> {
        self.list_tasks(|status| status.is_retriable())
    }

    /// Get prover reference
    pub fn prover(&self) -> &Arc<P> {
        &self.prover
    }

    /// Get configuration
    pub fn config(&self) -> &PaaSConfig<P::Backend> {
        &self.config
    }

    /// Get in-progress counter
    pub fn in_progress_counter(&self) -> &Arc<Mutex<HashMap<P::Backend, usize>>> {
        &self.in_progress
    }

    /// Generate status summary
    pub fn generate_summary(&self) -> StatusSummary {
        let tasks = self.tasks.lock().unwrap();

        let mut summary = StatusSummary {
            total: tasks.len(),
            pending: 0,
            queued: 0,
            proving: 0,
            completed: 0,
            transient_failure: 0,
            permanent_failure: 0,
        };

        for task in tasks.values() {
            match task.status {
                TaskStatus::Pending => summary.pending += 1,
                TaskStatus::Queued => summary.queued += 1,
                TaskStatus::Proving => summary.proving += 1,
                TaskStatus::Completed => summary.completed += 1,
                TaskStatus::TransientFailure { .. } => summary.transient_failure += 1,
                TaskStatus::PermanentFailure { .. } => summary.permanent_failure += 1,
            }
        }

        summary
    }
}

impl<P: Prover> ServiceState for ProverServiceState<P> {
    fn name(&self) -> &str {
        "prover_service"
    }
}

/// Status summary for monitoring
#[derive(Debug, Clone, Serialize)]
pub struct StatusSummary {
    pub total: usize,
    pub pending: usize,
    pub queued: usize,
    pub proving: usize,
    pub completed: usize,
    pub transient_failure: usize,
    pub permanent_failure: usize,
}
