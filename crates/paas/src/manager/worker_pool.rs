//! Worker pool for proof generation
//!
//! This module monitors ProverService for pending/retriable tasks and
//! dispatches them to workers for actual proof generation.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use strata_db::traits::ProofDatabase;
use strata_primitives::proof::ProofZkVm;
use tokio::{spawn, time::sleep};
use tracing::{debug, error, info};

use crate::{PaaSConfig, ProverHandle};

/// Proof operator trait that workers use to generate proofs
///
/// This is implemented by prover-client's ProofOperator
pub trait ProofOperatorTrait<D: ProofDatabase>: Send + Sync + 'static {
    /// Process a proof for the given proof key
    ///
    /// The operator is responsible for generating the proof and storing it in the database.
    /// Task status updates are handled by the WorkerPool.
    fn process_proof(
        &self,
        proof_key: strata_primitives::proof::ProofKey,
        db: &D,
    ) -> impl std::future::Future<Output = Result<(), Box<dyn std::error::Error + Send + Sync>>> + Send;
}

/// Worker pool service that processes proof tasks
#[derive(Debug)]
pub struct WorkerPool<D: ProofDatabase, O: ProofOperatorTrait<D>> {
    /// Handle to ProverService
    prover_handle: Arc<ProverHandle<D>>,
    /// Proof operator for generating proofs
    operator: Arc<O>,
    /// Database for storing proofs
    database: Arc<D>,
    /// Configuration
    config: PaaSConfig,
    /// Track in-progress tasks per backend (shared with spawned tasks)
    in_progress_tasks: Arc<Mutex<HashMap<ProofZkVm, usize>>>,
}

impl<D: ProofDatabase, O: ProofOperatorTrait<D>> WorkerPool<D, O> {
    /// Creates a new worker pool
    pub fn new(
        prover_handle: Arc<ProverHandle<D>>,
        operator: Arc<O>,
        database: Arc<D>,
        config: PaaSConfig,
    ) -> Self {
        Self {
            prover_handle,
            operator,
            database,
            config,
            in_progress_tasks: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Main processing loop
    pub async fn run(self) {
        info!("Worker pool started");

        loop {
            // Get pending tasks
            let pending_tasks = match self.prover_handle.list_pending_tasks().await {
                Ok(tasks) => tasks,
                Err(e) => {
                    error!(?e, "Failed to list pending tasks");
                    sleep(Duration::from_millis(
                        self.config.workers.polling_interval_ms,
                    ))
                    .await;
                    continue;
                }
            };

            // Get retriable tasks
            let retriable_tasks = match self.prover_handle.list_retriable_tasks().await {
                Ok(tasks) => tasks,
                Err(e) => {
                    error!(?e, "Failed to list retriable tasks");
                    sleep(Duration::from_millis(
                        self.config.workers.polling_interval_ms,
                    ))
                    .await;
                    continue;
                }
            };

            // Process pending tasks first, then retriable
            for task_id in pending_tasks.into_iter().chain(retriable_tasks) {
                // Get proof key for task
                let proof_key = match self.prover_handle.get_proof_key(task_id).await {
                    Ok(key) => key,
                    Err(e) => {
                        error!(?e, ?task_id, "Failed to get proof key");
                        continue;
                    }
                };

                let vm = proof_key.host();

                // Check worker limits and increment counter atomically
                let total_workers = self
                    .config
                    .workers
                    .worker_count
                    .get(vm)
                    .copied()
                    .unwrap_or(0);

                let should_skip = {
                    let mut tasks = self.in_progress_tasks.lock().unwrap();
                    let in_progress = tasks.get(vm).copied().unwrap_or(0);

                    if in_progress >= total_workers {
                        true
                    } else {
                        // Increment in-progress counter
                        *tasks.entry(*vm).or_insert(0) += 1;
                        false
                    }
                };

                if should_skip {
                    debug!(?proof_key, "Worker limit reached, skipping task");
                    continue;
                }

                // Clone resources for async task
                let operator = self.operator.clone();
                let database = self.database.clone();
                let prover_handle = self.prover_handle.clone();
                let in_progress_tasks = self.in_progress_tasks.clone();
                let vm_for_task = *vm;

                // Spawn proof generation task
                spawn(async move {
                    info!(?task_id, ?proof_key, "Starting proof generation");

                    // Ensure counter is decremented when task completes (success or failure)
                    let _guard = TaskCompletionGuard::new(in_progress_tasks.clone(), vm_for_task);

                    // Transition: Pending → Queued → Proving
                    if let Err(e) = prover_handle.mark_queued(task_id).await {
                        error!(?task_id, ?e, "Failed to mark task as queued");
                        return;
                    }

                    if let Err(e) = prover_handle.mark_proving(task_id).await {
                        error!(?task_id, ?e, "Failed to mark task as proving");
                        return;
                    }

                    let result = operator.process_proof(proof_key, database.as_ref()).await;

                    match result {
                        Ok(()) => {
                            info!(?task_id, "Proof generation completed");
                            if let Err(e) = prover_handle.mark_completed(task_id).await {
                                error!(?task_id, ?e, "Failed to mark task as completed");
                            }
                        }
                        Err(e) => {
                            let error_msg = e.to_string();
                            error!(?task_id, ?error_msg, "Proof generation failed");

                            // TODO: Classify errors into transient vs permanent failures:
                            // - Transient: Network timeouts, temporary resource exhaustion, DB
                            //   locks
                            // - Permanent: Invalid proof context, unsupported VM type, malformed
                            //   data
                            // For now, conservatively treat all errors as transient (will retry)
                            if let Err(e) = prover_handle
                                .mark_transient_failure(task_id, error_msg)
                                .await
                            {
                                error!(?task_id, ?e, "Failed to mark task as failed");
                            }
                        }
                    }
                    // _guard drops here, decrementing the counter
                });
            }

            // Sleep before next iteration
            sleep(Duration::from_millis(
                self.config.workers.polling_interval_ms,
            ))
            .await;
        }
    }
}

/// RAII guard that automatically decrements the in-progress task counter when dropped
///
/// This ensures the counter is decremented regardless of how the task completes
/// (success, error, or panic).
struct TaskCompletionGuard {
    in_progress_tasks: Arc<Mutex<HashMap<ProofZkVm, usize>>>,
    vm: ProofZkVm,
}

impl TaskCompletionGuard {
    fn new(in_progress_tasks: Arc<Mutex<HashMap<ProofZkVm, usize>>>, vm: ProofZkVm) -> Self {
        Self {
            in_progress_tasks,
            vm,
        }
    }
}

impl Drop for TaskCompletionGuard {
    fn drop(&mut self) {
        let mut tasks = self.in_progress_tasks.lock().unwrap();
        if let Some(count) = tasks.get_mut(&self.vm) {
            *count = count.saturating_sub(1);
            debug!(vm = ?self.vm, new_count = *count, "Task completed, decremented worker counter");
        }
    }
}
