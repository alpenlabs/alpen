//! Adapter to bridge old TaskTracker interface with new PaaS ProverHandle

use std::{collections::HashMap, sync::Arc};

use strata_db_store_sled::prover::ProofDBSled;
use strata_paas::{ProverHandle, TaskId};
use strata_primitives::proof::{ProofContext, ProofKey, ProofZkVm};
use tracing::{error, warn};

use crate::{errors::ProvingTaskError, operators::TaskTrackerLike, status::ProvingTaskStatus};

/// Adapter that provides old TaskTracker interface using PaaS ProverHandle
pub(crate) struct TaskTrackerAdapter {
    prover_handle: Arc<ProverHandle<ProofDBSled>>,
    /// Map ProofKey to TaskId for reverse lookups
    key_to_id: Arc<tokio::sync::Mutex<HashMap<ProofKey, TaskId>>>,
    /// List of ZkVm backends
    vms: Vec<ProofZkVm>,
}

impl TaskTrackerAdapter {
    pub(crate) fn new(prover_handle: Arc<ProverHandle<ProofDBSled>>) -> Self {
        let mut vms = vec![];

        #[cfg(feature = "sp1")]
        {
            vms.push(ProofZkVm::SP1);
        }

        #[cfg(not(feature = "sp1"))]
        {
            vms.push(ProofZkVm::Native);
        }

        Self {
            prover_handle,
            key_to_id: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            vms,
        }
    }

    /// Creates tasks for the given proof context and dependencies
    /// Returns ProofKeys for compatibility with existing operators
    ///
    /// Note: This adapter now handles dependencies by waiting for them to complete
    /// before creating the main task. This ensures PaaS remains dependency-agnostic.
    pub(crate) async fn create_tasks(
        &mut self,
        proof_id: ProofContext,
        deps: Vec<ProofContext>,
        db: &ProofDBSled,
    ) -> Result<Vec<ProofKey>, ProvingTaskError> {
        tracing::info!(?proof_id, ?deps, "Creating task with dependencies");

        // Wait for all dependencies to complete first
        if !deps.is_empty() {
            self.wait_for_dependencies(&deps, db).await?;
        }

        let mut tasks = Vec::with_capacity(self.vms.len());
        let mut key_to_id = self.key_to_id.lock().await;

        // Insert tasks for each configured host
        for host in &self.vms {
            let proof_key = ProofKey::new(proof_id, *host);

            // Check if task already exists
            if key_to_id.contains_key(&proof_key) {
                return Err(ProvingTaskError::TaskAlreadyFound(proof_key));
            }

            // Create task using ProverHandle (no deps passed to PaaS)
            let task_id = self
                .prover_handle
                .create_task(proof_id)
                .await
                .map_err(|e| ProvingTaskError::RpcError(e.to_string()))?;

            // Store mapping
            key_to_id.insert(proof_key, task_id);
            tasks.push(proof_key);
        }

        Ok(tasks)
    }

    /// Waits for all dependencies to complete
    ///
    /// This method ensures that all dependencies are completed before creating
    /// the main task, effectively handling dependency management at the caller level.
    async fn wait_for_dependencies(
        &self,
        deps: &[ProofContext],
        db: &ProofDBSled,
    ) -> Result<(), ProvingTaskError> {
        use strata_db::traits::ProofDatabase;
        use tokio::time::{Duration, sleep};

        tracing::info!(?deps, "Waiting for dependencies to complete");

        // Check if all dependencies have completed proofs in the database
        let mut pending_deps: Vec<_> = deps.to_vec();

        // Retry loop with timeout
        let max_wait = Duration::from_secs(3600); // 1 hour max
        let poll_interval = Duration::from_millis(500);
        let start = std::time::Instant::now();

        while !pending_deps.is_empty() {
            if start.elapsed() > max_wait {
                return Err(ProvingTaskError::DependencyTimeout(format!(
                    "Timeout waiting for dependencies: {:?}",
                    pending_deps
                )));
            }

            // Check each pending dependency
            pending_deps.retain(|dep_context| {
                let vm = self.vms[0]; // Use primary VM
                let dep_key = ProofKey::new(*dep_context, vm);

                match db.get_proof(&dep_key) {
                    Ok(Some(_)) => {
                        tracing::info!(?dep_context, "Dependency completed");
                        false // Remove from pending
                    }
                    Ok(None) => {
                        true // Still pending
                    }
                    Err(e) => {
                        tracing::warn!(?e, ?dep_context, "Error checking dependency");
                        true // Treat errors as still pending
                    }
                }
            });

            if !pending_deps.is_empty() {
                sleep(poll_interval).await;
            }
        }

        tracing::info!("All dependencies completed");
        Ok(())
    }

    /// Gets the status of a task by ProofKey
    pub(crate) async fn get_task(
        &self,
        key: ProofKey,
    ) -> Result<ProvingTaskStatus, ProvingTaskError> {
        let key_to_id = self.key_to_id.lock().await;
        let task_id = key_to_id
            .get(&key)
            .ok_or(ProvingTaskError::TaskNotFound(key))?;

        let status = self
            .prover_handle
            .get_task_status(*task_id)
            .await
            .map_err(|e| ProvingTaskError::RpcError(e.to_string()))?;

        // Convert PaaS TaskStatus to old ProvingTaskStatus
        Ok(convert_status(status))
    }

    /// Generates a report of task counts by status
    pub(crate) async fn generate_report(&self) -> HashMap<String, usize> {
        match self.prover_handle.list_tasks(None).await {
            Ok(tasks) => {
                let mut report = HashMap::new();
                for (_id, status) in tasks {
                    let status_str = format!("{:?}", convert_status(status));
                    *report.entry(status_str).or_insert(0) += 1;
                }
                report
            }
            Err(e) => {
                error!(?e, "Failed to generate report");
                HashMap::new()
            }
        }
    }

    /// Updates task status (for compatibility, but PaaS manages this internally)
    pub(crate) async fn update_status(
        &mut self,
        key: ProofKey,
        new_status: ProvingTaskStatus,
        _max_retry_counter: u64,
    ) -> Result<(), ProvingTaskError> {
        let key_to_id = self.key_to_id.lock().await;
        let task_id = key_to_id
            .get(&key)
            .ok_or(ProvingTaskError::TaskNotFound(key))?;

        // Map status updates to PaaS operations
        use strata_paas::TaskStatusUpdate;

        match new_status {
            ProvingTaskStatus::Completed => {
                self.prover_handle
                    .set_task_status(*task_id, TaskStatusUpdate::Completed)
                    .await
                    .map_err(|e| ProvingTaskError::RpcError(e.to_string()))?;

                // Clean up mapping for completed tasks
                drop(key_to_id);
                self.key_to_id.lock().await.remove(&key);
            }
            ProvingTaskStatus::TransientFailure => {
                self.prover_handle
                    .set_task_status(
                        *task_id,
                        TaskStatusUpdate::TransientFailure {
                            error: "Worker reported transient failure".to_string(),
                        },
                    )
                    .await
                    .map_err(|e| ProvingTaskError::RpcError(e.to_string()))?;
            }
            ProvingTaskStatus::Failed => {
                self.prover_handle
                    .set_task_status(
                        *task_id,
                        TaskStatusUpdate::Failed {
                            error: "Worker reported permanent failure".to_string(),
                        },
                    )
                    .await
                    .map_err(|e| ProvingTaskError::RpcError(e.to_string()))?;
            }
            _ => {
                warn!(?new_status, "Unsupported status update in adapter");
            }
        }

        Ok(())
    }

    /// Clears internal state (for testing)
    pub(crate) async fn clear_state(&mut self) {
        self.key_to_id.lock().await.clear();
    }

    /// Gets tasks by status filter
    pub(crate) async fn get_tasks_by_status<F>(&self, filter: F) -> Vec<ProofKey>
    where
        F: Fn(&ProvingTaskStatus) -> bool,
    {
        let key_to_id = self.key_to_id.lock().await;
        let mut result = Vec::new();

        for (proof_key, task_id) in key_to_id.iter() {
            if let Ok(status) = self.prover_handle.get_task_status(*task_id).await {
                let old_status = convert_status(status);
                if filter(&old_status) {
                    result.push(*proof_key);
                }
            }
        }

        result
    }

    /// Gets retriable tasks (transient failures with retry count)
    pub(crate) async fn get_retriable_tasks(&self) -> HashMap<ProofKey, u64> {
        let key_to_id = self.key_to_id.lock().await;
        let mut retriable = HashMap::new();

        for (proof_key, task_id) in key_to_id.iter() {
            if let Ok(status) = self.prover_handle.get_task_status(*task_id).await {
                if let strata_paas::TaskStatus::TransientFailure { retry_count, .. } = status {
                    retriable.insert(*proof_key, retry_count as u64);
                }
            }
        }

        retriable
    }

    /// Gets tasks waiting for dependencies
    pub(crate) async fn get_waiting_for_dependencies_tasks(&self) -> Vec<ProofKey> {
        self.get_tasks_by_status(|s| matches!(s, ProvingTaskStatus::WaitingForDependencies))
            .await
    }

    /// Gets in-progress task counts by VM
    pub(crate) async fn get_in_progress_tasks(&self) -> HashMap<ProofZkVm, usize> {
        let key_to_id = self.key_to_id.lock().await;
        let mut counts: HashMap<ProofZkVm, usize> = HashMap::new();

        for (proof_key, task_id) in key_to_id.iter() {
            if let Ok(status) = self.prover_handle.get_task_status(*task_id).await {
                if matches!(convert_status(status), ProvingTaskStatus::ProvingInProgress) {
                    *counts.entry(*proof_key.host()).or_insert(0) += 1;
                }
            }
        }

        counts
    }
}

/// Implement TaskTrackerLike trait for TaskTrackerAdapter
impl TaskTrackerLike for TaskTrackerAdapter {
    async fn create_tasks(
        &mut self,
        proof_id: ProofContext,
        deps: Vec<ProofContext>,
        _db: &ProofDBSled,
    ) -> Result<Vec<ProofKey>, ProvingTaskError> {
        self.create_tasks(proof_id, deps, _db).await
    }
}

/// Converts PaaS TaskStatus to old ProvingTaskStatus
fn convert_status(status: strata_paas::TaskStatus) -> ProvingTaskStatus {
    match status {
        strata_paas::TaskStatus::Pending => ProvingTaskStatus::Pending,
        strata_paas::TaskStatus::Queued => ProvingTaskStatus::Pending,
        strata_paas::TaskStatus::Proving { .. } => ProvingTaskStatus::ProvingInProgress,
        strata_paas::TaskStatus::Completed { .. } => ProvingTaskStatus::Completed,
        strata_paas::TaskStatus::Failed { .. } => ProvingTaskStatus::Failed,
        strata_paas::TaskStatus::TransientFailure { .. } => ProvingTaskStatus::TransientFailure,
        strata_paas::TaskStatus::Cancelled { .. } => ProvingTaskStatus::Failed,
    }
}
