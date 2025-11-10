//! Service state management for PaaS

use std::sync::Arc;

use strata_db::traits::ProofDatabase;
use strata_service::ServiceState;
use tokio::sync::watch;

use crate::{
    PaaSConfig, PaaSError, PaaSReport, PaaSStatus, TaskId, TaskStatus,
    commands::{ProofData, TaskStatusFilter},
    manager::TaskTracker,
};

/// Internal state for ProverService
#[derive(Debug)]
pub struct ProverServiceState<D: ProofDatabase> {
    /// Configuration
    config: PaaSConfig,
    /// Task tracker for managing task lifecycle
    task_tracker: TaskTracker,
    /// Proof database for storage
    database: Arc<D>,
    /// Status channel for broadcasting service status
    status_tx: watch::Sender<PaaSStatus>,
    /// Cumulative statistics
    stats: CumulativeStats,
}

/// Cumulative statistics (persisted across task lifecycles)
#[derive(Debug, Default)]
struct CumulativeStats {
    total_completed: usize,
    total_failed: usize,
    total_proofs: u64,
    completed_proofs: u64,
    failed_proofs: u64,
}

impl<D: ProofDatabase> ProverServiceState<D> {
    /// Creates a new service state
    pub fn new(config: PaaSConfig, database: Arc<D>, status_tx: watch::Sender<PaaSStatus>) -> Self {
        Self {
            config,
            task_tracker: TaskTracker::new(),
            database,
            status_tx,
            stats: CumulativeStats::default(),
        }
    }

    /// Gets the task tracker
    pub fn task_tracker(&self) -> &TaskTracker {
        &self.task_tracker
    }

    /// Gets mutable task tracker
    pub fn task_tracker_mut(&mut self) -> &mut TaskTracker {
        &mut self.task_tracker
    }

    /// Gets the proof database
    pub fn database(&self) -> &Arc<D> {
        &self.database
    }

    /// Gets the configuration
    pub fn config(&self) -> &PaaSConfig {
        &self.config
    }

    /// Creates a new proof task
    ///
    /// Note: Caller is responsible for managing dependencies.
    /// Dependent tasks should only be created after their dependencies are completed.
    pub fn create_task(
        &mut self,
        context: strata_primitives::proof::ProofContext,
    ) -> Result<TaskId, PaaSError> {
        self.stats.total_proofs += 1;
        let task_id = self
            .task_tracker
            .create_task(context, self.database.as_ref())?;
        self.update_status();
        Ok(task_id)
    }

    /// Gets task status
    pub fn get_task_status(&self, task_id: TaskId) -> Result<TaskStatus, PaaSError> {
        self.task_tracker.get_task_status(task_id)
    }

    /// Gets proof data for a completed task
    pub fn get_proof(&self, task_id: TaskId) -> Result<Option<ProofData>, PaaSError> {
        let proof_key = self.task_tracker.get_proof_key(task_id)?;

        let proof_receipt = self
            .database
            .get_proof(&proof_key)
            .map_err(|e| PaaSError::Storage(e.to_string()))?;

        // Convert zkaleido ProofReceiptWithMetadata to our ProofData
        Ok(proof_receipt.map(|receipt| {
            // Serialize the entire ProofReceiptWithMetadata using borsh
            let receipt_bytes = borsh::to_vec(&receipt)
                .expect("ProofReceiptWithMetadata should serialize successfully");

            ProofData {
                receipt: receipt_bytes,
                public_values: None,    // TODO: Extract if needed by clients
                verification_key: None, // TODO: Extract if needed by clients
            }
        }))
    }

    /// Cancels a task
    ///
    /// Note: Currently only validates task existence. Full cancellation requires:
    /// - Worker pool coordination to stop in-flight proofs
    /// - Graceful cleanup of resources
    /// - State transition to Cancelled status
    pub fn cancel_task(&mut self, task_id: TaskId) -> Result<(), PaaSError> {
        // Verify task exists
        let _ = self.task_tracker.get_task_status(task_id)?;
        // TODO: Implement actual cancellation logic (requires worker pool coordination)
        Ok(())
    }

    /// Lists tasks with optional filter
    pub fn list_tasks(
        &self,
        filter: Option<TaskStatusFilter>,
    ) -> Result<Vec<(TaskId, TaskStatus)>, PaaSError> {
        let task_ids: Vec<TaskId> = match filter {
            None => {
                // Would need to iterate all - not supported by current API
                // For now, combine pending + queued + proving
                let mut all = self.task_tracker.list_pending();
                all.extend(self.task_tracker.list_queued());
                all.extend(self.task_tracker.list_proving());
                all
            }
            Some(TaskStatusFilter::Pending) => self.task_tracker.list_pending(),
            Some(TaskStatusFilter::Queued) => self.task_tracker.list_queued(),
            Some(TaskStatusFilter::Proving) => self.task_tracker.list_proving(),
            Some(TaskStatusFilter::Active) => {
                let mut active = self.task_tracker.list_queued();
                active.extend(self.task_tracker.list_proving());
                active
            }
            Some(TaskStatusFilter::TransientFailure) => self
                .task_tracker
                .get_retriable_tasks()
                .keys()
                .copied()
                .collect(),
            Some(
                TaskStatusFilter::Failed
                | TaskStatusFilter::Completed
                | TaskStatusFilter::Cancelled,
            ) => {
                // These are removed from tracker when reached
                vec![]
            }
        };

        // Convert to (TaskId, TaskStatus) pairs
        let mut tasks = Vec::new();
        for task_id in task_ids {
            if let Ok(status) = self.task_tracker.get_task_status(task_id) {
                tasks.push((task_id, status));
            }
        }

        Ok(tasks)
    }

    /// Gets proof key for a task
    pub fn get_proof_key(
        &self,
        task_id: TaskId,
    ) -> Result<strata_primitives::proof::ProofKey, PaaSError> {
        self.task_tracker.get_proof_key(task_id)
    }

    /// Generates a service report
    pub fn generate_report(&self) -> PaaSReport {
        PaaSReport {
            total_proofs: self.stats.total_proofs,
            completed_proofs: self.stats.completed_proofs,
            failed_proofs: self.stats.failed_proofs,
            // TODO: Track task durations (requires storing start_time in TaskTracker
            // and calculating elapsed time on completion)
            average_duration_ms: 0,
            worker_stats: crate::status::WorkerStats {
                total_workers: self.config.workers.worker_count.values().sum(),
                busy_workers: self.task_tracker.get_stats().proving,
                available_workers: self
                    .config
                    .workers
                    .worker_count
                    .values()
                    .sum::<usize>()
                    .saturating_sub(self.task_tracker.get_stats().proving),
            },
        }
    }

    /// Records task completion
    pub fn record_completion(&mut self, _task_id: TaskId) {
        self.stats.total_completed += 1;
        self.stats.completed_proofs += 1;
        self.update_status();
    }

    /// Records task failure
    pub fn record_failure(&mut self, _task_id: TaskId) {
        self.stats.total_failed += 1;
        self.stats.failed_proofs += 1;
        self.update_status();
    }

    /// Updates and broadcasts current status
    pub fn update_status(&self) {
        let status = self.get_status();
        let _ = self.status_tx.send(status);
    }

    /// Marks a task as queued (ready to prove)
    pub fn mark_queued(&mut self, task_id: TaskId) -> Result<(), PaaSError> {
        use crate::manager::task_tracker::InternalTaskStatus;
        self.task_tracker.update_status(
            task_id,
            InternalTaskStatus::Queued,
            self.config.retry.max_retries,
        )?;
        self.update_status();
        Ok(())
    }

    /// Marks a task as proving/in-progress
    pub fn mark_proving(&mut self, task_id: TaskId) -> Result<(), PaaSError> {
        use crate::manager::task_tracker::InternalTaskStatus;
        self.task_tracker.update_status(
            task_id,
            InternalTaskStatus::Proving,
            self.config.retry.max_retries,
        )?;
        self.update_status();
        Ok(())
    }

    /// Marks a task as completed
    pub fn mark_completed(&mut self, task_id: TaskId) -> Result<(), PaaSError> {
        use crate::manager::task_tracker::InternalTaskStatus;
        self.task_tracker.update_status(
            task_id,
            InternalTaskStatus::Completed,
            self.config.retry.max_retries,
        )?;
        self.record_completion(task_id);
        Ok(())
    }

    /// Marks a task as having a transient failure (will retry)
    pub fn mark_transient_failure(
        &mut self,
        task_id: TaskId,
        _error: &str,
    ) -> Result<(), PaaSError> {
        use crate::manager::task_tracker::InternalTaskStatus;
        self.task_tracker.update_status(
            task_id,
            InternalTaskStatus::TransientFailure,
            self.config.retry.max_retries,
        )?;
        self.update_status();
        Ok(())
    }

    /// Marks a task as permanently failed
    pub fn mark_failed(&mut self, task_id: TaskId, _error: &str) -> Result<(), PaaSError> {
        use crate::manager::task_tracker::InternalTaskStatus;
        self.task_tracker.update_status(
            task_id,
            InternalTaskStatus::Failed,
            self.config.retry.max_retries,
        )?;
        self.record_failure(task_id);
        Ok(())
    }
}

impl<D: ProofDatabase> ServiceState for ProverServiceState<D> {
    fn name(&self) -> &str {
        "prover_service"
    }
}

impl<D: ProofDatabase> ProverServiceState<D> {
    pub fn get_status(&self) -> PaaSStatus {
        let stats = self.task_tracker.get_stats();
        let total_workers: usize = self.config.workers.worker_count.values().sum();
        let utilization = if total_workers > 0 {
            stats.proving as f32 / total_workers as f32
        } else {
            0.0
        };

        PaaSStatus {
            active_tasks: stats.proving,
            queued_tasks: stats.queued,
            completed_tasks: self.stats.total_completed,
            failed_tasks: self.stats.total_failed,
            worker_utilization: utilization,
        }
    }
}
