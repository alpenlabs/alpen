//! Task tracking and state management for proof generation
//!
//! This module manages the lifecycle of proof tasks, including dependency
//! resolution, retry logic, and state transitions.

use std::collections::{HashMap, HashSet, hash_map::Entry};

use strata_db::traits::ProofDatabase;
use strata_primitives::proof::{ProofContext, ProofKey, ProofZkVm};
use tracing::info;
use uuid::Uuid;

use crate::{PaaSError, TaskId, TaskStatus};

/// Manages tasks and their states for proving operations
#[derive(Debug)]
pub struct TaskTracker {
    /// Map of TaskId (UUID) to ProofKey
    task_id_to_key: HashMap<TaskId, ProofKey>,
    /// Map of ProofKey to TaskId (reverse mapping)
    key_to_task_id: HashMap<ProofKey, TaskId>,
    /// Map of TaskIds to their statuses with metadata
    tasks: HashMap<TaskId, TaskInfo>,
    /// Map of TaskIds that have failed transiently to their retry counter
    transient_failed_tasks: HashMap<TaskId, u32>,
    /// Map of TaskIds to their dependencies that have not yet been proven
    pending_dependencies: HashMap<TaskId, HashSet<TaskId>>,
    /// Count of the tasks that are in progress per backend
    in_progress_tasks: HashMap<ProofZkVm, usize>,
    /// List of ZkVm backends configured
    vms: Vec<ProofZkVm>,
}

/// Internal task information with metadata
#[derive(Debug, Clone)]
struct TaskInfo {
    status: InternalTaskStatus,
    context: ProofContext,
    deps: Vec<ProofContext>,
}

/// Internal task status (simpler than public TaskStatus)
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum InternalTaskStatus {
    WaitingForDependencies,
    Pending,
    Queued,
    Proving,
    Completed,
    TransientFailure,
    Failed,
}

impl InternalTaskStatus {
    /// Attempts to transition to new status
    fn transition(&mut self, target: InternalTaskStatus) -> Result<(), PaaSError> {
        let is_valid = match (self.clone(), &target) {
            // Always allow transitioning to Failed
            (_, InternalTaskStatus::Failed) => true,

            // Normal flow
            (InternalTaskStatus::Pending, InternalTaskStatus::Queued) => true,
            (InternalTaskStatus::Queued, InternalTaskStatus::Proving) => true,
            (InternalTaskStatus::Proving, InternalTaskStatus::Completed) => true,
            (InternalTaskStatus::WaitingForDependencies, InternalTaskStatus::Pending) => true,

            // Transient failure flow
            (InternalTaskStatus::Proving, InternalTaskStatus::TransientFailure) => true,
            (InternalTaskStatus::TransientFailure, InternalTaskStatus::Queued) => true,

            _ => false,
        };

        if is_valid {
            *self = target;
            Ok(())
        } else {
            Err(PaaSError::Unexpected(format!(
                "invalid status transition from {:?} to {:?}",
                self, target
            )))
        }
    }
}

impl Default for TaskTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl TaskTracker {
    /// Creates a new TaskTracker instance
    pub fn new() -> Self {
        let mut vms = vec![];

        #[cfg(feature = "sp1")]
        {
            vms.push(ProofZkVm::SP1);
        }

        #[cfg(not(feature = "sp1"))]
        {
            vms.push(ProofZkVm::Native);
        }

        TaskTracker {
            task_id_to_key: HashMap::new(),
            key_to_task_id: HashMap::new(),
            tasks: HashMap::new(),
            transient_failed_tasks: HashMap::new(),
            pending_dependencies: HashMap::new(),
            in_progress_tasks: HashMap::new(),
            vms,
        }
    }

    /// Creates a new task with dependencies
    pub fn create_task<D: ProofDatabase>(
        &mut self,
        context: ProofContext,
        deps: Vec<ProofContext>,
        db: &D,
    ) -> Result<TaskId, PaaSError> {
        info!(?context, "Creating task for proof context");

        let task_id = Uuid::new_v4();

        // Create ProofKey for the primary VM
        let vm = self.vms[0];
        let proof_key = ProofKey::new(context, vm);

        // Check if proof already exists
        if let Some(_proof) = db
            .get_proof(&proof_key)
            .map_err(|e| PaaSError::Storage(e.to_string()))?
        {
            return Err(PaaSError::TaskAlreadyExists(task_id));
        }

        // Store mappings
        self.task_id_to_key.insert(task_id, proof_key);
        self.key_to_task_id.insert(proof_key, task_id);

        // Process dependencies
        let mut pending_deps = Vec::new();
        let mut dep_task_ids = Vec::new();

        for dep_context in &deps {
            let dep_key = ProofKey::new(*dep_context, vm);

            // Check if dependency proof exists
            let proof = db
                .get_proof(&dep_key)
                .map_err(|e| PaaSError::Storage(e.to_string()))?;

            if proof.is_none() {
                // Dependency not completed, need to track it
                pending_deps.push(dep_key);

                // Try to find existing task for this dependency
                if let Some(&dep_task_id) = self.key_to_task_id.get(&dep_key) {
                    dep_task_ids.push(dep_task_id);
                } else {
                    // Dependency doesn't exist as a task - this is an error
                    return Err(PaaSError::InvalidContext(format!(
                        "dependency {:?} does not exist as a task",
                        dep_context
                    )));
                }
            }
        }

        // Determine initial status
        let status = if dep_task_ids.is_empty() {
            InternalTaskStatus::Pending
        } else {
            self.pending_dependencies
                .insert(task_id, HashSet::from_iter(dep_task_ids));
            InternalTaskStatus::WaitingForDependencies
        };

        // Store task info
        self.tasks.insert(
            task_id,
            TaskInfo {
                status,
                context,
                deps,
            },
        );

        Ok(task_id)
    }

    /// Retrieves the status of a task
    pub fn get_task_status(&self, task_id: TaskId) -> Result<TaskStatus, PaaSError> {
        let info = self
            .tasks
            .get(&task_id)
            .ok_or(PaaSError::TaskNotFound(task_id))?;

        // Convert internal status to public TaskStatus
        let status = match &info.status {
            InternalTaskStatus::Pending => TaskStatus::Pending,
            InternalTaskStatus::Queued => TaskStatus::Queued,
            InternalTaskStatus::Proving => TaskStatus::Proving {
                progress: 0.5,
                started_at: chrono::Utc::now(),
            },
            InternalTaskStatus::Completed => TaskStatus::Completed {
                completed_at: chrono::Utc::now(),
                duration_ms: 0,
            },
            InternalTaskStatus::Failed => TaskStatus::Failed {
                failed_at: chrono::Utc::now(),
                error: "Task failed".to_string(),
                retry_count: *self.transient_failed_tasks.get(&task_id).unwrap_or(&0),
            },
            InternalTaskStatus::TransientFailure => TaskStatus::TransientFailure {
                failed_at: chrono::Utc::now(),
                error: "Transient failure".to_string(),
                retry_count: *self.transient_failed_tasks.get(&task_id).unwrap_or(&0),
                next_retry_at: chrono::Utc::now(),
            },
            InternalTaskStatus::WaitingForDependencies => TaskStatus::Pending,
        };

        Ok(status)
    }

    /// Updates the status of a task
    pub(crate) fn update_status(
        &mut self,
        task_id: TaskId,
        new_status: InternalTaskStatus,
        max_retries: u32,
    ) -> Result<(), PaaSError> {
        let info = self
            .tasks
            .get_mut(&task_id)
            .ok_or(PaaSError::TaskNotFound(task_id))?;

        // Perform state transition
        info.status.transition(new_status.clone())?;

        // Get ProofKey for this task
        let proof_key = self
            .task_id_to_key
            .get(&task_id)
            .ok_or(PaaSError::TaskNotFound(task_id))?;
        let vm = proof_key.host();

        // Handle side effects based on new status
        match new_status {
            InternalTaskStatus::Proving => {
                *self.in_progress_tasks.entry(*vm).or_insert(0) += 1;
            }
            InternalTaskStatus::Completed => {
                // Decrement in-progress count
                if let Some(count) = self.in_progress_tasks.get_mut(vm) {
                    *count = count.saturating_sub(1);
                }

                // Resolve dependencies for other tasks
                let mut tasks_to_update = vec![];
                for (dependent_task, deps) in self.pending_dependencies.iter_mut() {
                    if deps.remove(&task_id) && deps.is_empty() {
                        tasks_to_update.push(*dependent_task);
                    }
                }

                for dep_task in tasks_to_update {
                    self.pending_dependencies.remove(&dep_task);
                    if let Some(task_info) = self.tasks.get_mut(&dep_task) {
                        let _ = task_info.status.transition(InternalTaskStatus::Pending);
                    }
                }

                // Clean up completed task
                self.tasks.remove(&task_id);
                self.transient_failed_tasks.remove(&task_id);
                self.task_id_to_key.remove(&task_id);
                if let Some(key) = self.task_id_to_key.get(&task_id) {
                    self.key_to_task_id.remove(key);
                }
            }
            InternalTaskStatus::TransientFailure => {
                // Decrement in-progress count
                if let Some(count) = self.in_progress_tasks.get_mut(vm) {
                    *count = count.saturating_sub(1);
                }

                // Check retry counter
                let retry_count = self.transient_failed_tasks.entry(task_id);
                match retry_count {
                    Entry::Occupied(mut entry) => {
                        if *entry.get() >= max_retries {
                            // Exceeded retry limit, mark as permanently failed
                            if let Some(task_info) = self.tasks.get_mut(&task_id) {
                                let _ = task_info.status.transition(InternalTaskStatus::Failed);
                            }
                            self.transient_failed_tasks.remove(&task_id);
                        } else {
                            *entry.get_mut() += 1;
                        }
                    }
                    Entry::Vacant(entry) => {
                        entry.insert(1);
                    }
                }
            }
            InternalTaskStatus::Failed => {
                // Clean up retry counter
                self.transient_failed_tasks.remove(&task_id);

                // Mark dependent tasks as failed
                for (dependent_task, deps) in self.pending_dependencies.iter_mut() {
                    if deps.remove(&task_id)
                        && let Some(task_info) = self.tasks.get_mut(dependent_task)
                    {
                        let _ = task_info.status.transition(InternalTaskStatus::Failed);
                    }
                }
            }
            _ => {}
        }

        Ok(())
    }

    /// Gets pending tasks ready to be queued
    pub fn get_pending_tasks(&self) -> Vec<TaskId> {
        self.tasks
            .iter()
            .filter_map(|(task_id, info)| {
                if info.status == InternalTaskStatus::Pending {
                    Some(*task_id)
                } else {
                    None
                }
            })
            .collect()
    }

    /// Gets tasks ready for retry
    pub fn get_retriable_tasks(&self) -> HashMap<TaskId, u32> {
        self.tasks
            .iter()
            .filter_map(|(task_id, info)| {
                if info.status == InternalTaskStatus::TransientFailure {
                    Some((
                        *task_id,
                        *self.transient_failed_tasks.get(task_id).unwrap_or(&0),
                    ))
                } else {
                    None
                }
            })
            .collect()
    }

    /// Gets current task statistics
    pub fn get_stats(&self) -> TaskStats {
        let mut stats = TaskStats {
            pending: 0,
            queued: 0,
            proving: 0,
            completed: 0,
            failed: 0,
            waiting_deps: 0,
        };

        for info in self.tasks.values() {
            match info.status {
                InternalTaskStatus::Pending => stats.pending += 1,
                InternalTaskStatus::Queued => stats.queued += 1,
                InternalTaskStatus::Proving => stats.proving += 1,
                InternalTaskStatus::Completed => stats.completed += 1,
                InternalTaskStatus::Failed => stats.failed += 1,
                InternalTaskStatus::WaitingForDependencies => stats.waiting_deps += 1,
                _ => {}
            }
        }

        stats
    }

    /// Gets the ProofKey for a task
    pub fn get_proof_key(&self, task_id: TaskId) -> Result<ProofKey, PaaSError> {
        self.task_id_to_key
            .get(&task_id)
            .copied()
            .ok_or(PaaSError::TaskNotFound(task_id))
    }

    /// Gets all pending tasks
    pub fn list_pending(&self) -> Vec<TaskId> {
        self.tasks
            .iter()
            .filter_map(|(id, info)| {
                if info.status == InternalTaskStatus::Pending {
                    Some(*id)
                } else {
                    None
                }
            })
            .collect()
    }

    /// Gets all queued tasks
    pub fn list_queued(&self) -> Vec<TaskId> {
        self.tasks
            .iter()
            .filter_map(|(id, info)| {
                if info.status == InternalTaskStatus::Queued {
                    Some(*id)
                } else {
                    None
                }
            })
            .collect()
    }

    /// Gets all proving tasks
    pub fn list_proving(&self) -> Vec<TaskId> {
        self.tasks
            .iter()
            .filter_map(|(id, info)| {
                if info.status == InternalTaskStatus::Proving {
                    Some(*id)
                } else {
                    None
                }
            })
            .collect()
    }

    /// Gets task context and dependencies
    pub fn get_task_info(
        &self,
        task_id: TaskId,
    ) -> Result<(ProofContext, Vec<ProofContext>), PaaSError> {
        let info = self
            .tasks
            .get(&task_id)
            .ok_or(PaaSError::TaskNotFound(task_id))?;
        Ok((info.context, info.deps.clone()))
    }
}

/// Task statistics
#[derive(Debug, Default)]
pub struct TaskStats {
    pub pending: usize,
    pub queued: usize,
    pub proving: usize,
    pub completed: usize,
    pub failed: usize,
    pub waiting_deps: usize,
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use strata_db_store_sled::{SledDbConfig, prover::ProofDBSled};
    use strata_primitives::{buf::Buf32, evm_exec::EvmEeBlockCommitment};
    use typed_sled::SledDb;

    use super::*;

    fn setup_db() -> ProofDBSled {
        let db = sled::Config::new().temporary(true).open().unwrap();
        let sled_db = Arc::new(SledDb::new(db).unwrap());
        let config = SledDbConfig::new_with_constant_backoff(3, 200);
        ProofDBSled::new(sled_db, config).unwrap()
    }

    #[test]
    fn test_create_task_no_dependencies() {
        let mut tracker = TaskTracker::new();
        let db = setup_db();

        let start = EvmEeBlockCommitment::new(1, Buf32::default());
        let end = EvmEeBlockCommitment::new(2, Buf32::default());
        let context = ProofContext::EvmEeStf(start, end);

        let task_id = tracker.create_task(context, vec![], &db).unwrap();
        let status = tracker.get_task_status(task_id).unwrap();

        assert_eq!(status, TaskStatus::Pending);
    }

    #[test]
    fn test_task_not_found() {
        let tracker = TaskTracker::new();
        let task_id = Uuid::new_v4();

        let result = tracker.get_task_status(task_id);
        assert!(matches!(result, Err(PaaSError::TaskNotFound(_))));
    }
}
