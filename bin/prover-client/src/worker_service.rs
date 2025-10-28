//! Worker pool service for processing proof tasks
//!
//! This service monitors the ProverService for pending tasks and dispatches
//! them to worker threads for actual proof generation.

use std::{collections::HashMap, sync::Arc, time::Duration};

use strata_db_store_sled::prover::ProofDBSled;
use strata_paas::{PaaSConfig, ProverHandle, TaskId, TaskStatus};
use strata_primitives::proof::{ProofContext, ProofKey, ProofZkVm};
use tokio::{spawn, time::sleep};
use tracing::{debug, error, info, warn};

use crate::{
    checkpoint_runner::errors::CheckpointError, errors::ProvingTaskError, operators::ProofOperator,
    retry_policy::ExponentialBackoff,
};

/// Worker pool service that processes proof tasks from ProverService
pub struct WorkerPoolService {
    /// Handle to ProverService
    prover_handle: ProverHandle<ProofDBSled>,
    /// Proof operator for generating proofs
    operator: Arc<ProofOperator>,
    /// Database for storing proofs
    db: Arc<ProofDBSled>,
    /// Configuration
    config: PaaSConfig,
    /// Retry policy
    retry_policy: ExponentialBackoff,
    /// Track in-progress tasks per backend
    in_progress_tasks: HashMap<ProofZkVm, usize>,
}

impl WorkerPoolService {
    /// Creates a new worker pool service
    pub fn new(
        prover_handle: ProverHandle<ProofDBSled>,
        operator: Arc<ProofOperator>,
        db: Arc<ProofDBSled>,
        config: PaaSConfig,
    ) -> Self {
        Self {
            prover_handle,
            operator,
            db,
            retry_policy: ExponentialBackoff::new(
                config.retry.max_retries as u64,
                config.retry.max_delay_secs,
                config.retry.multiplier,
            ),
            config,
            in_progress_tasks: HashMap::new(),
        }
    }

    /// Main processing loop
    pub async fn run(mut self) {
        info!("Worker pool service started");

        loop {
            // Get current status to find tasks needing processing
            let status = self.prover_handle.status();

            // For now, we don't have a way to query pending tasks from ProverHandle
            // This is a limitation we'll need to address in Phase 2
            // TODO: Add API to ProverHandle to list pending/retriable tasks

            debug!(
                active_tasks = status.active_tasks,
                queued_tasks = status.queued_tasks,
                "Worker pool status"
            );

            // Sleep before next iteration
            sleep(Duration::from_millis(
                self.config.workers.polling_interval_ms,
            ))
            .await;
        }
    }

    /// Processes a single proof task
    async fn process_task(&mut self, task_id: TaskId, proof_key: ProofKey, retry_count: u32) {
        let vm = proof_key.host();

        // Check worker limits
        let total_workers = self
            .config
            .workers
            .worker_count
            .get(vm)
            .copied()
            .unwrap_or(0);
        let in_progress = self.in_progress_tasks.get(vm).copied().unwrap_or(0);

        if in_progress >= total_workers {
            warn!(?proof_key, "Worker limit reached, skipping task");
            return;
        }

        // Increment in-progress counter
        *self.in_progress_tasks.entry(*vm).or_insert(0) += 1;

        // Calculate retry delay
        let retry_delay = self.retry_policy.get_delay(retry_count as u64);

        // Clone resources for async task
        let operator = self.operator.clone();
        let db = self.db.clone();
        let prover_handle = self.prover_handle.clone();

        // Spawn proof generation task
        spawn(async move {
            if let Err(err) =
                Self::make_proof(operator, prover_handle, task_id, proof_key, db, retry_delay)
                    .await
            {
                error!(?err, ?task_id, "Failed to process proof task");
            }
        });
    }

    /// Generates a proof for the given task
    async fn make_proof(
        operator: Arc<ProofOperator>,
        prover_handle: ProverHandle<ProofDBSled>,
        task_id: TaskId,
        proof_key: ProofKey,
        db: Arc<ProofDBSled>,
        delay_seconds: u64,
    ) -> Result<(), ProvingTaskError> {
        // Handle retry delay
        if delay_seconds > 0 {
            debug!(
                ?task_id,
                ?delay_seconds,
                "Scheduling transiently failed task to run after delay"
            );
            sleep(Duration::from_secs(delay_seconds)).await;
        }

        info!(?task_id, ?proof_key, "Starting proof generation");

        // Check if proof already exists
        let mut proving_task_res = {
            if let Ok(Some(_)) = db.get_proof(&proof_key) {
                info!(?task_id, "Proof already exists in database");
                Ok(())
            } else {
                operator.process_proof(&proof_key, &db).await
            }
        };

        // If checkpoint, submit to sequencer
        if let ProofContext::Checkpoint(checkpoint_index, ..) = proof_key.context() {
            if proving_task_res.is_ok() {
                proving_task_res = operator
                    .checkpoint_operator()
                    .submit_checkpoint_proof(*checkpoint_index, &proof_key, &db)
                    .await
                    .map_err(handle_checkpoint_error);
            }
        }

        // Determine task outcome
        match proving_task_res {
            Ok(_) => {
                info!(?task_id, "Proof generation completed successfully");
                // TODO: Update task status via ProverHandle
                // For now, we rely on ProverService to track completion
                Ok(())
            }
            Err(e) => {
                error!(?task_id, ?e, "Proof generation failed");
                // TODO: Report failure to ProverService for retry logic
                Err(e)
            }
        }
    }
}

/// Handles checkpoint submission errors
fn handle_checkpoint_error(chkpt_err: CheckpointError) -> ProvingTaskError {
    match chkpt_err {
        CheckpointError::FetchError(error) => ProvingTaskError::RpcError(error),
        CheckpointError::SubmitProofError { error, .. } => {
            if error.to_lowercase().contains("proof already created") {
                ProvingTaskError::IdempotentCompletion(error)
            } else {
                ProvingTaskError::RpcError(error)
            }
        }
        CheckpointError::ProofErr(proving_task_error) => proving_task_error,
    }
}
