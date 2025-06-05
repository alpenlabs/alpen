use std::{collections::HashMap, sync::Arc, time::Duration};

use strata_db::traits::ProofDatabase;
use strata_primitives::proof::{ProofContext, ProofKey, ProofZkVm};
use strata_rocksdb::prover::db::ProofDb;
use tokio::{spawn, sync::Mutex, time::sleep};
use tracing::{error, info, warn};

use crate::{
    checkpoint_runner::errors::CheckpointError, errors::ProvingTaskError, operators::ProofOperator,
    retry_policy::ExponentialBackoff, status::ProvingTaskStatus, task_tracker::TaskTracker,
};

/// Configuration for the [`ProverManager`].
#[derive(Debug, Clone)]
pub(crate) struct ProverManagerConfig {
    /// Worker configuration for different proving backends.
    pub(crate) workers: HashMap<ProofZkVm, usize>,

    /// Polling interval for the prover manager loop in milliseconds.
    pub(crate) loop_interval: u64,

    /// Maximum number of retries for transient failures.
    pub(crate) max_retry_counter: u64,
}

impl ProverManagerConfig {
    /// Creates a new [`ProverManagerConfig`] with the given parameters.
    pub(crate) fn new(
        workers: HashMap<ProofZkVm, usize>,
        loop_interval: u64,
        max_retry_counter: u64,
    ) -> Self {
        Self {
            workers,
            loop_interval,
            max_retry_counter,
        }
    }
}

/// The prover manager is responsible for managing the proving tasks and dispatching them to the
/// appropriate proving backends.
#[derive(Debug, Clone)]
pub(crate) struct ProverManager {
    /// Task tracker for the prover manager.
    pub(crate) task_tracker: Arc<Mutex<TaskTracker>>,

    /// Operator for the prover manager.
    pub(crate) operator: Arc<ProofOperator>,

    /// Database for the prover manager.
    pub(crate) db: Arc<ProofDb>,

    /// Configuration for the prover manager.
    pub(crate) config: ProverManagerConfig,

    /// Retry policy for the prover manager.
    pub(crate) retry_policy: ExponentialBackoff,
}

impl ProverManager {
    /// Creates a new [`ProverManager`] with the given parameters.
    pub(crate) fn new(
        task_tracker: Arc<Mutex<TaskTracker>>,
        operator: Arc<ProofOperator>,
        db: Arc<ProofDb>,
        config: ProverManagerConfig,
    ) -> Self {
        Self {
            task_tracker,
            operator,
            db,
            retry_policy: ExponentialBackoff::new(
                config.max_retry_counter,
                3_600, /* one hour total time */
                1.5,
            ),
            config,
        }
    }

    /// Processes the pending tasks.
    pub(crate) async fn process_pending_tasks(&self) {
        loop {
            // Step 1: Fetch tasks data without holding the lock
            let (pending_tasks, _, retriable_tasks, mut in_progress_tasks) = {
                let task_tracker = self.task_tracker.lock().await;
                let pending_tasks = task_tracker
                    .get_tasks_by_status(|status| matches!(status, ProvingTaskStatus::Pending));
                (
                    pending_tasks,
                    task_tracker.get_waiting_for_dependencies_tasks().clone(),
                    task_tracker.get_retriable_tasks().clone(),
                    task_tracker.get_in_progress_tasks().clone(),
                )
            };

            // Step 2: Process each task:
            // We chain pending tasks (with retry=0) and retriable_tasks (with retry > 0) into one
            // iterator, and process using the same mechanism.
            // P.S. The pending tasks go first.
            for (task, retry) in pending_tasks
                .into_iter()
                .map(|task| (task, 0))
                .chain(retriable_tasks.into_iter())
            {
                let total_workers = self.config.workers.get(task.host()).unwrap_or(&0);
                let in_progress_workers = in_progress_tasks.get(task.host()).unwrap_or(&0);

                // Skip tasks if worker limit is reached
                if in_progress_workers >= total_workers {
                    warn!(?task, "Worker limit reached, skipping task");
                    continue;
                }
                *in_progress_tasks.entry(*task.host()).or_insert(0) += 1;

                // First, transition the task to be in progress, so it's not picked up by the
                // next iterations in this loop.
                {
                    let mut task_tracker = self.task_tracker.lock().await;
                    if let Err(err) = task_tracker.update_status(
                        task,
                        ProvingTaskStatus::ProvingInProgress,
                        self.config.max_retry_counter,
                    ) {
                        error!(?err, "Failed to transition task to in progress.")
                    }
                }

                // Clone resources for async task
                let operator = self.operator.clone();
                let db = self.db.clone();
                let task_tracker = self.task_tracker.clone();
                let max_retry_counter = self.config.max_retry_counter;
                // Calculate the delay
                let retry_delay = self.retry_policy.get_delay(retry);

                // Spawn a new task with delay.
                spawn(async move {
                    if let Err(err) = make_proof(
                        operator,
                        task_tracker,
                        task,
                        db,
                        retry_delay,
                        max_retry_counter,
                    )
                    .await
                    {
                        error!(?err, "Failed to process task");
                    }
                });
            }

            // Step 3: Sleep before the next loop iteration
            sleep(Duration::from_millis(self.config.loop_interval)).await;
        }
    }
}

/// Dispatches the given task to do all the proving routine and handles the status and errors.
async fn make_proof(
    operator: Arc<ProofOperator>,
    task_tracker: Arc<Mutex<TaskTracker>>,
    task: ProofKey,
    db: Arc<ProofDb>,
    delay_seconds: u64,
    max_retry_counter: u64,
) -> Result<(), ProvingTaskError> {
    // Handle the delay (if set) from the TransientFailure retries.
    if delay_seconds > 0 {
        info!(
            "scheduling transiently failed task {:?} to run in {} seconds",
            task, delay_seconds
        );
        sleep(Duration::from_secs(delay_seconds)).await;
    }

    info!(?task, ?delay_seconds, "start proving the task");

    // Check if the proof already exists and do the proving only if it doesn't.
    // N.B. Currently, it can only happen if checkpoint submit is being retried.
    let mut proving_task_res = {
        if let Ok(Some(_)) = db.get_proof(&task) {
            Ok(())
        } else {
            operator.process_proof(&task, &db).await
        }
    };

    // If the task is a Checkpoint, try to submit checkpoint proof back to the sequencer.
    if let ProofContext::Checkpoint(checkpoint_index, ..) = task.context() {
        if proving_task_res.is_ok() {
            proving_task_res = operator
                .checkpoint_operator()
                .submit_checkpoint_proof(*checkpoint_index, &task, &db)
                .await
                .map_err(handle_checkpoint_error);
        }
    }

    // Determine the next status for the task given the result.
    let new_status = match proving_task_res {
        Ok(_) => ProvingTaskStatus::Completed,
        Err(e) => handle_task_error(task, e),
    };

    // Update the task status.
    {
        let mut task_tracker = task_tracker.lock().await;
        task_tracker.update_status(task, new_status, max_retry_counter)
    }
}

/// Handles the task error by determining the next status based on [`ProvingTaskError`] nature.
fn handle_task_error(task: ProofKey, e: ProvingTaskError) -> ProvingTaskStatus {
    match e {
        ProvingTaskError::RpcError(_) => {
            // RpcError is retryable as it usually indicates the downstream services may
            // currently be unavailable.
            error!(?task, ?e, "proving task failed transiently");
            ProvingTaskStatus::TransientFailure
        }
        ProvingTaskError::ZkVmError(zkaleido::ZkVmError::ProofGenerationError(ref message)) => {
            if message.to_lowercase().contains("unavailable") {
                // This type of error with status:Unavailable indicates network error on SP1 side.
                // See STR-1410 for details.
                error!(?task, ?e, "proving task failed transiently");
                ProvingTaskStatus::TransientFailure
            } else {
                error!(?task, ?e, "proving task failed");
                ProvingTaskStatus::Failed
            }
        }
        ProvingTaskError::ProofAlreadyExists => {
            // Proof already exists, which is a success case
            warn!(?task, "Proof already exists, marking as completed");
            ProvingTaskStatus::Completed
        }
        _ => {
            // Other errors are treated as non-retryable, so the task is failed permanently.
            error!(?task, ?e, "proving task failed");
            ProvingTaskStatus::Failed
        }
    }
}

/// Handles the checkpoint submit error by converting it to the appropriate [`ProvingTaskError`].
/// Then, the [`ProvingTaskError`] is handled as usual.
fn handle_checkpoint_error(chkpt_err: CheckpointError) -> ProvingTaskError {
    match chkpt_err {
        CheckpointError::FetchError(error) => ProvingTaskError::RpcError(error),
        CheckpointError::SubmitProofError { error, .. } => {
            if error.to_lowercase().contains("proof already created") {
                ProvingTaskError::ProofAlreadyExists
            } else {
                ProvingTaskError::RpcError(error)
            }
        }
        CheckpointError::CheckpointNotFound(_) => ProvingTaskError::WitnessNotFound,
        CheckpointError::ProofErr(proving_task_error) => proving_task_error,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use strata_primitives::proof::{ProofContext, ProofZkVm};
    use strata_rpc_types::ProofKey;

    use super::*;

    #[test]
    fn test_prover_manager_config_creation() {
        let mut workers = HashMap::new();
        workers.insert(ProofZkVm::Native, 5);
        workers.insert(ProofZkVm::SP1, 10);

        let config = ProverManagerConfig::new(
            workers.clone(),
            1_000, // 1 second polling interval
            15,    // max retry counter
        );

        assert_eq!(config.workers, workers);
        assert_eq!(config.loop_interval, 1_000);
        assert_eq!(config.max_retry_counter, 15);
    }

    #[test]
    fn test_transient_retry_rpc_error() {
        let mock_key = ProofKey::new(
            ProofContext::Checkpoint(0),
            strata_primitives::proof::ProofZkVm::SP1,
        );
        let err = ProvingTaskError::RpcError("rpc error".to_string());

        assert_eq!(
            handle_task_error(mock_key, err),
            ProvingTaskStatus::TransientFailure
        );
    }

    #[test]
    fn test_transient_retry_zkvm_unavailable() {
        let mock_key = ProofKey::new(
            ProofContext::Checkpoint(0),
            strata_primitives::proof::ProofZkVm::SP1,
        );
        let err = ProvingTaskError::ZkVmError(zkaleido::ZkVmError::ProofGenerationError(
            "Unavailable".to_string(),
        ));

        assert_eq!(
            handle_task_error(mock_key, err),
            ProvingTaskStatus::TransientFailure
        );
    }

    #[test]
    fn test_failed_zkvm_panic() {
        let mock_key = ProofKey::new(
            ProofContext::Checkpoint(0),
            strata_primitives::proof::ProofZkVm::SP1,
        );
        let err = ProvingTaskError::ZkVmError(zkaleido::ZkVmError::ProofGenerationError(
            "panic during proof generation".to_string(),
        ));

        assert_eq!(handle_task_error(mock_key, err), ProvingTaskStatus::Failed);
    }

    #[test]
    fn test_proof_already_exists_completed() {
        let mock_key = ProofKey::new(
            ProofContext::Checkpoint(0),
            strata_primitives::proof::ProofZkVm::SP1,
        );
        let err = ProvingTaskError::ProofAlreadyExists;

        assert_eq!(
            handle_task_error(mock_key, err),
            ProvingTaskStatus::Completed
        );
    }

    #[test]
    fn test_checkpoint_error_proof_already_created() {
        use crate::checkpoint_runner::errors::CheckpointError;

        let err = CheckpointError::SubmitProofError {
            index: 0,
            error: "Proof already created for checkpoint 0".to_string(),
        };

        match handle_checkpoint_error(err) {
            ProvingTaskError::ProofAlreadyExists => {
                // Expected case
            }
            other => panic!("Expected ProofAlreadyExists, got {other:?}"),
        }
    }

    #[test]
    fn test_checkpoint_error_other_submit_errors() {
        use crate::checkpoint_runner::errors::CheckpointError;

        let err = CheckpointError::SubmitProofError {
            index: 0,
            error: "Network timeout".to_string(),
        };

        match handle_checkpoint_error(err) {
            ProvingTaskError::RpcError(msg) => {
                assert_eq!(msg, "Network timeout");
            }
            other => panic!("Expected RpcError, got {other:?}"),
        }
    }

    #[test]
    fn test_checkpoint_error_fetch_error() {
        use crate::checkpoint_runner::errors::CheckpointError;

        let err = CheckpointError::FetchError("Connection failed".to_string());

        match handle_checkpoint_error(err) {
            ProvingTaskError::RpcError(msg) => {
                assert_eq!(msg, "Connection failed");
            }
            other => panic!("Expected RpcError, got {other:?}"),
        }
    }
}
