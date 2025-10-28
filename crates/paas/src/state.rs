//! Service state management for PaaS

use std::sync::Arc;

use strata_db::traits::ProofDatabase;
use strata_service::ServiceState;
use tokio::sync::watch;

use crate::{
    commands::ProofData,
    manager::TaskTracker,
    PaaSConfig, PaaSError, PaaSReport, PaaSStatus, TaskId, TaskStatus,
};

/// Internal state for ProverService
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
    pub fn create_task(
        &mut self,
        context: strata_primitives::proof::ProofContext,
        deps: Vec<strata_primitives::proof::ProofContext>,
    ) -> Result<TaskId, PaaSError> {
        self.stats.total_proofs += 1;
        let task_id = self
            .task_tracker
            .create_task(context, deps, self.database.as_ref())?;
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
        Ok(proof_receipt.map(|_receipt| {
            // TODO: Properly serialize ProofReceiptWithMetadata
            // For now, return placeholder data
            ProofData {
                receipt: vec![],
                public_values: None,
                verification_key: None,
            }
        }))
    }

    /// Cancels a task
    pub fn cancel_task(&mut self, task_id: TaskId) -> Result<(), PaaSError> {
        // For now, just verify task exists
        let _ = self.task_tracker.get_task_status(task_id)?;
        // TODO: Implement actual cancellation logic
        Ok(())
    }

    /// Generates a service report
    pub fn generate_report(&self) -> PaaSReport {
        PaaSReport {
            total_proofs: self.stats.total_proofs,
            completed_proofs: self.stats.completed_proofs,
            failed_proofs: self.stats.failed_proofs,
            average_duration_ms: 0, // TODO: Track durations
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

    /// Updates and broadcasts current status
    pub fn update_status(&self) {
        let status = self.get_status();
        let _ = self.status_tx.send(status);
    }
}
