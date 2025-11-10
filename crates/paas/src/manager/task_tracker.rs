//! Task tracking and state management for proof generation
//!
//! This module manages the lifecycle of proof tasks, including dependency
//! resolution, retry logic, and state transitions.

use std::collections::{HashMap, hash_map::Entry};

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
}

/// Internal task status (simpler than public TaskStatus)
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum InternalTaskStatus {
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
            in_progress_tasks: HashMap::new(),
            vms,
        }
    }

    /// Creates a new task without dependencies
    ///
    /// Caller is responsible for ensuring dependencies are completed before creating this task
    pub fn create_task<D: ProofDatabase>(
        &mut self,
        context: ProofContext,
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

        // Store task info - always starts as Pending
        self.tasks.insert(
            task_id,
            TaskInfo {
                status: InternalTaskStatus::Pending,
                context,
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
                // Note: Dependency management is caller's responsibility
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
        };

        for info in self.tasks.values() {
            match info.status {
                InternalTaskStatus::Pending => stats.pending += 1,
                InternalTaskStatus::Queued => stats.queued += 1,
                InternalTaskStatus::Proving => stats.proving += 1,
                InternalTaskStatus::Completed => stats.completed += 1,
                InternalTaskStatus::Failed => stats.failed += 1,
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

    /// Gets task context
    pub fn get_task_context(&self, task_id: TaskId) -> Result<ProofContext, PaaSError> {
        let info = self
            .tasks
            .get(&task_id)
            .ok_or(PaaSError::TaskNotFound(task_id))?;
        Ok(info.context)
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
    fn test_create_task() {
        let mut tracker = TaskTracker::new();
        let db = setup_db();

        let start = EvmEeBlockCommitment::new(1, Buf32::default());
        let end = EvmEeBlockCommitment::new(2, Buf32::default());
        let context = ProofContext::EvmEeStf(start, end);

        let task_id = tracker.create_task(context, &db).unwrap();
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
