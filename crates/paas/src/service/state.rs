//! Service state management with persistence

use std::{collections::HashMap, sync::Arc};

use serde::{Deserialize, Serialize};
use strata_service::ServiceState;
use strata_tasks::TaskExecutor;
use tokio::sync::Semaphore;
use tracing::{debug, error, info, warn};

use crate::{
    config::ProverServiceConfig,
    error::{ProverServiceError, ProverServiceResult},
    handler::ProofHandler,
    persistence::{TaskRecord, TaskStore},
    program::ProgramType,
    scheduler::SchedulerHandle,
    task::{TaskId, TaskResult, TaskStatus},
    ZkVmBackend,
};

/// Service state for ProverService (Direct Handler Execution with Persistence)
///
/// This state manages:
/// - Task lifecycle management with persistent tracking
/// - Direct handler dispatch for proof execution
/// - Semaphore-based capacity control per backend
/// - Idempotent task submission via TaskStore
///
/// Handlers encapsulate execution complexity (!Send futures, LocalSet, etc.)
pub struct ProverServiceState<P: ProgramType> {
    /// Configuration
    pub(crate) config: ProverServiceConfig<ZkVmBackend>,

    /// Persistent task storage
    pub(crate) task_store: Arc<dyn TaskStore<P>>,

    /// Handlers for each program variant
    pub(crate) handlers: HashMap<P::RoutingKey, Arc<dyn ProofHandler<P>>>,

    /// Semaphores for capacity control per backend
    pub(crate) semaphores: HashMap<ZkVmBackend, Arc<Semaphore>>,

    /// Task executor for spawning background tasks
    pub(crate) executor: TaskExecutor,

    /// Retry scheduler handle for delayed executions
    pub(crate) retry_scheduler: SchedulerHandle<P>,
}

impl<P: ProgramType> std::fmt::Debug for ProverServiceState<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProverServiceState")
            .field("config", &self.config)
            .field("handler_count", &self.handlers.len())
            .field("retry_scheduler", &self.retry_scheduler)
            .finish()
    }
}

impl<P: ProgramType> ProverServiceState<P> {
    /// Create new service state with handlers and semaphores
    pub fn new(
        config: ProverServiceConfig<ZkVmBackend>,
        task_store: Arc<dyn TaskStore<P>>,
        handlers: HashMap<P::RoutingKey, Arc<dyn ProofHandler<P>>>,
        semaphores: HashMap<ZkVmBackend, Arc<Semaphore>>,
        executor: TaskExecutor,
        retry_scheduler: SchedulerHandle<P>,
    ) -> Self {
        Self {
            config,
            task_store,
            handlers,
            semaphores,
            executor,
            retry_scheduler,
        }
    }

    /// Submit a new task with idempotent UUID generation
    ///
    /// This method is idempotent: if the same TaskId is submitted multiple times,
    /// it returns the existing UUID instead of creating a new entry.
    ///
    /// # Idempotency Guarantees
    ///
    /// - Same TaskId → Same UUID (persisted mapping)
    /// - No duplicate task execution
    /// - Safe to retry on network failures
    /// - Race-safe: concurrent submits of same TaskId handled correctly
    ///
    /// # Returns
    ///
    /// Returns (UUID, is_new) where is_new indicates if this is a newly created task
    pub fn submit_task(&self, task_id: TaskId<P>) -> ProverServiceResult<(String, bool)> {
        // Check if task already exists (idempotency check)
        if let Some(uuid) = self.task_store.get_uuid(&task_id) {
            return Ok((uuid, false));
        }

        // Generate UUID for new task
        let uuid = uuid::Uuid::new_v4().to_string();

        let record = TaskRecord::new(task_id.clone(), uuid.clone(), TaskStatus::Pending);

        // Persist the task - handle race condition where another thread inserted first
        match self.task_store.insert_task(record) {
            Ok(()) => Ok((uuid, true)),
            Err(ProverServiceError::Config(ref msg)) if msg.contains("already exists") => {
                // Race condition: another thread inserted between our check and insert
                // Fetch the existing UUID instead
                if let Some(existing_uuid) = self.task_store.get_uuid(&task_id) {
                    Ok((existing_uuid, false))
                } else {
                    // UUID should exist but doesn't - database inconsistency
                    Err(ProverServiceError::Internal(anyhow::anyhow!(
                        "Task exists but UUID not found: {:?}",
                        task_id
                    )))
                }
            }
            Err(e) => Err(e),
        }
    }

    /// Get task status by UUID
    pub fn get_status_by_uuid(&self, uuid: &str) -> ProverServiceResult<TaskStatus> {
        let record = self
            .task_store
            .get_task_by_uuid(uuid)
            .ok_or_else(|| ProverServiceError::TaskNotFound(format!("UUID: {}", uuid)))?;

        Ok(record.status().clone())
    }

    /// Get task status by TaskId (internal use)
    pub fn get_status(&self, task_id: &TaskId<P>) -> ProverServiceResult<TaskStatus> {
        let record = self
            .task_store
            .get_task(task_id)
            .ok_or_else(|| ProverServiceError::TaskNotFound(format!("{:?}", task_id)))?;

        Ok(record.status().clone())
    }

    /// Update task status
    pub fn update_status(
        &self,
        task_id: &TaskId<P>,
        status: TaskStatus,
    ) -> ProverServiceResult<()> {
        self.task_store.update_status(task_id, status)
    }

    /// List tasks by status filter
    pub fn list_tasks(&self, filter: Box<dyn Fn(&TaskStatus) -> bool + '_>) -> Vec<TaskId<P>> {
        self.task_store
            .list_tasks(filter)
            .into_iter()
            .map(|record| record.task_id().clone())
            .collect()
    }

    /// List pending tasks
    pub fn list_pending(&self) -> Vec<TaskId<P>> {
        self.list_tasks(Box::new(|status| matches!(status, TaskStatus::Pending)))
    }

    /// List retriable tasks
    pub fn list_retriable(&self) -> Vec<TaskId<P>> {
        self.list_tasks(Box::new(|status| status.is_retriable()))
    }

    /// Get configuration
    pub fn config(&self) -> &ProverServiceConfig<ZkVmBackend> {
        &self.config
    }

    /// Generate status summary
    pub fn generate_summary(&self) -> StatusSummary {
        let all_tasks = self.task_store.list_tasks(Box::new(|_| true));

        let mut summary = StatusSummary {
            total: all_tasks.len(),
            pending: 0,
            queued: 0,
            proving: 0,
            completed: 0,
            transient_failure: 0,
            permanent_failure: 0,
        };

        for record in all_tasks {
            match record.status() {
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

    /// Execute a task using the appropriate handler
    ///
    /// This method performs the actual proof generation:
    /// 1. Fetch input via handler
    /// 2. Execute proof with semaphore control
    /// 3. Store completed proof
    ///
    /// Returns Ok(()) on success, Err for transient/permanent failures.
    #[tracing::instrument(skip(self))]
    async fn execute_task(&self, task_id: &TaskId<P>) -> ProverServiceResult<()> {
        // Get handler for this program variant
        let routing_key = task_id.program().routing_key();
        let handler = self.handlers.get(&routing_key).ok_or_else(|| {
            ProverServiceError::PermanentFailure(format!(
                "No handler registered for program variant: {:?}",
                routing_key
            ))
        })?;

        // Get semaphore for backend
        let backend = task_id.backend();
        let semaphore = self.semaphores.get(backend).ok_or_else(|| {
            ProverServiceError::PermanentFailure(format!(
                "No semaphore configured for backend: {:?}",
                backend
            ))
        })?;

        debug!("Fetching input");
        let input = handler.fetch_input(task_id.program()).await?;

        // Acquire semaphore for capacity control (blocks if at limit)
        debug!(?backend, "Acquiring capacity permit");
        let _permit = semaphore.acquire().await.map_err(|e| {
            ProverServiceError::Internal(anyhow::anyhow!("Semaphore closed: {}", e))
        })?;

        debug!("Executing proof with capacity permit held");
        let proof = handler
            .execute_proof(task_id.program(), input, backend)
            .await?;

        debug!("Storing proof");
        handler.store_proof(task_id.program(), proof).await?;

        // Permit automatically released when dropped
        Ok(())
    }

    /// Execute a task synchronously and return the result (AWAITABLE)
    ///
    /// This method is for awaitable execution - it blocks until the task completes
    /// and returns the result. Does NOT spawn in background, does NOT retry on failure.
    ///
    /// Use this when you need to wait for task completion (e.g., checkpoint runner).
    /// Use execute_and_track() for fire-and-forget with retry logic.
    ///
    /// # Returns
    ///
    /// - `TaskResult::Completed` if proof generation succeeded
    /// - `TaskResult::Failed` if proof generation failed
    #[tracing::instrument(skip(self), fields(uuid))]
    pub async fn execute_task_sync(&self, task_id: TaskId<P>) -> TaskResult {
        // Submit task to get UUID (idempotent)
        let (uuid, is_new) = match self.submit_task(task_id.clone()) {
            Ok(result) => result,
            Err(e) => {
                error!(?e, "Failed to submit task");
                return TaskResult::Failed {
                    uuid: "unknown".to_string(),
                    error: format!("Submit failed: {}", e),
                };
            }
        };

        // Record uuid in span
        tracing::Span::current().record("uuid", &uuid);

        // If task already exists, check its status
        if !is_new {
            match self.get_status(&task_id) {
                Ok(TaskStatus::Completed) => {
                    debug!("Task already completed");
                    return TaskResult::Completed { uuid };
                }
                Ok(TaskStatus::PermanentFailure { error }) => {
                    warn!(%error, "Task already failed");
                    return TaskResult::Failed { uuid, error };
                }
                Ok(status) => {
                    debug!(?status, "Task exists, waiting for current execution");
                    // Task is in progress, we'll wait for it by executing again
                }
                Err(e) => {
                    warn!(?e, "Failed to get task status");
                }
            }
        }

        // Transition to Queued
        if let Err(e) = self.update_status(&task_id, TaskStatus::Queued) {
            error!(?e, "Failed to update status to Queued");
            return TaskResult::Failed {
                uuid: uuid.clone(),
                error: format!("Status update failed: {}", e),
            };
        }
        debug!("Task queued");

        // Transition to Proving
        if let Err(e) = self.update_status(&task_id, TaskStatus::Proving) {
            error!(?e, "Failed to update status to Proving");
            return TaskResult::Failed {
                uuid: uuid.clone(),
                error: format!("Status update failed: {}", e),
            };
        }
        info!("Task proving started");

        // Execute the task
        match self.execute_task(&task_id).await {
            Ok(()) => {
                // Success - mark as completed
                if let Err(e) = self.update_status(&task_id, TaskStatus::Completed) {
                    error!(?e, "Failed to update status to Completed");
                    return TaskResult::Failed {
                        uuid: uuid.clone(),
                        error: format!("Status update failed: {}", e),
                    };
                }
                info!("Task completed successfully");
                TaskResult::Completed { uuid }
            }
            Err(e) => {
                // Failure - mark as failed
                let error_msg = e.to_string();
                error!(%error_msg, "Task failed");

                if let Err(update_err) = self.update_status(
                    &task_id,
                    TaskStatus::PermanentFailure {
                        error: error_msg.clone(),
                    },
                ) {
                    error!(?update_err, "Failed to update status to PermanentFailure");
                }

                TaskResult::Failed {
                    uuid,
                    error: error_msg,
                }
            }
        }
    }

    /// Execute a task and track its status through all transitions
    ///
    /// This method orchestrates the full task lifecycle:
    /// - Pending → Queued → Proving → Completed/Failed
    /// - Handles errors and retry logic
    /// - Updates status at each transition
    ///
    /// Should be called in a spawned task to avoid blocking.
    pub async fn execute_and_track(&self, task_id: TaskId<P>) {
        // Transition to Queued
        if let Err(e) = self.update_status(&task_id, TaskStatus::Queued) {
            error!(?task_id, ?e, "Failed to update status to Queued");
            return; // Cannot proceed without status tracking
        }
        info!(?task_id, "Task queued");

        // Transition to Proving
        if let Err(e) = self.update_status(&task_id, TaskStatus::Proving) {
            error!(?task_id, ?e, "Failed to update status to Proving");
            return; // Cannot proceed without status tracking
        }
        info!(?task_id, "Task proving started");

        // Execute via handler
        match self.execute_task(&task_id).await {
            Ok(()) => {
                // Success - mark as completed
                if let Err(e) = self.update_status(&task_id, TaskStatus::Completed) {
                    error!(?task_id, ?e, "Failed to update status to Completed - task succeeded but status not persisted");
                } else {
                    info!(?task_id, "Task completed successfully");
                }
            }
            Err(ProverServiceError::TransientFailure(msg)) => {
                warn!(?task_id, %msg, "Task failed with transient error");

                // Check if retries are enabled
                if let Some(ref retry_config) = self.config.retry {
                    // Get current retry count
                    let retry_count = self.get_retry_count(&task_id);
                    let new_retry_count = retry_count + 1;

                    if retry_config.should_retry(new_retry_count) {
                        // Update status with retry count
                        if let Err(e) = self.update_status(
                            &task_id,
                            TaskStatus::TransientFailure {
                                retry_count: new_retry_count,
                                error: msg.clone(),
                            },
                        ) {
                            error!(?task_id, ?e, "Failed to update status to TransientFailure");
                        } else {
                            info!(
                                ?task_id,
                                retry_count = new_retry_count,
                                "Task marked for retry"
                            );
                        }

                        // Schedule retry after delay
                        self.schedule_retry(task_id, new_retry_count);
                    } else {
                        // Max retries exceeded - mark as permanent failure
                        let error_msg = format!("Max retries exceeded: {}", msg);
                        if let Err(e) = self.update_status(
                            &task_id,
                            TaskStatus::PermanentFailure {
                                error: error_msg.clone(),
                            },
                        ) {
                            error!(?task_id, ?e, "Failed to update status to PermanentFailure");
                        } else {
                            error!(?task_id, %error_msg, "Task permanently failed after max retries");
                        }
                    }
                } else {
                    // Retries disabled - mark as permanent failure
                    let error_msg = format!("Retries disabled: {}", msg);
                    if let Err(e) = self.update_status(
                        &task_id,
                        TaskStatus::PermanentFailure {
                            error: error_msg.clone(),
                        },
                    ) {
                        error!(?task_id, ?e, "Failed to update status to PermanentFailure");
                    } else {
                        warn!(?task_id, %error_msg, "Task permanently failed (retries disabled)");
                    }
                }
            }
            Err(ProverServiceError::PermanentFailure(msg)) => {
                // Permanent failure - don't retry
                error!(?task_id, %msg, "Task permanently failed");
                if let Err(e) =
                    self.update_status(&task_id, TaskStatus::PermanentFailure { error: msg })
                {
                    error!(?task_id, ?e, "Failed to update status to PermanentFailure");
                }
            }
            Err(e) => {
                // Unknown error - treat as transient if retries enabled
                warn!(?task_id, ?e, "Task failed with unknown error");

                if self.config.retry.is_some() {
                    // Retries enabled - treat as transient error
                    if let Err(err) = self.update_status(
                        &task_id,
                        TaskStatus::TransientFailure {
                            retry_count: 1,
                            error: e.to_string(),
                        },
                    ) {
                        error!(
                            ?task_id,
                            ?err,
                            "Failed to update status to TransientFailure"
                        );
                    } else {
                        info!(?task_id, "Task marked for retry after unknown error");
                    }

                    // Schedule retry
                    self.schedule_retry(task_id, 1);
                } else {
                    // Retries disabled - mark as permanent failure
                    let error_msg = format!("Retries disabled: {}", e);
                    if let Err(err) = self.update_status(
                        &task_id,
                        TaskStatus::PermanentFailure {
                            error: error_msg.clone(),
                        },
                    ) {
                        error!(
                            ?task_id,
                            ?err,
                            "Failed to update status to PermanentFailure"
                        );
                    } else {
                        warn!(?task_id, %error_msg, "Task permanently failed (retries disabled)");
                    }
                }
            }
        }
    }

    /// Schedule a retry for a task after the calculated delay
    ///
    /// This is a non-async method that delegates to the retry scheduler.
    /// The scheduler will send a RetryTask command back to the service after the delay.
    fn schedule_retry(&self, task_id: TaskId<P>, retry_count: u32) {
        if let Some(ref retry_config) = self.config.retry {
            let delay_secs = retry_config.calculate_delay(retry_count);
            self.retry_scheduler.schedule_retry(task_id, delay_secs);
        }
    }

    /// Get the current retry count for a task
    fn get_retry_count(&self, task_id: &TaskId<P>) -> u32 {
        if let Ok(TaskStatus::TransientFailure { retry_count, .. }) = self.get_status(task_id) {
            retry_count
        } else {
            0
        }
    }
}

// Implement Clone for ProverServiceState (required by ServiceState)
impl<P: ProgramType> Clone for ProverServiceState<P> {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            task_store: self.task_store.clone(),
            handlers: self.handlers.clone(),
            semaphores: self.semaphores.clone(),
            executor: self.executor.clone(),
            retry_scheduler: self.retry_scheduler.clone(),
        }
    }
}

impl<P: ProgramType> ServiceState for ProverServiceState<P> {
    fn name(&self) -> &str {
        "prover_service"
    }
}

/// Status summary for monitoring
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusSummary {
    pub total: usize,
    pub pending: usize,
    pub queued: usize,
    pub proving: usize,
    pub completed: usize,
    pub transient_failure: usize,
    pub permanent_failure: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_status_summary_serialization() {
        let summary = StatusSummary {
            total: 10,
            pending: 2,
            queued: 1,
            proving: 3,
            completed: 2,
            transient_failure: 1,
            permanent_failure: 1,
        };

        let json = serde_json::to_string(&summary).unwrap();
        let deserialized: StatusSummary = serde_json::from_str(&json).unwrap();

        assert_eq!(summary.total, deserialized.total);
        assert_eq!(summary.pending, deserialized.pending);
        assert_eq!(summary.queued, deserialized.queued);
        assert_eq!(summary.proving, deserialized.proving);
        assert_eq!(summary.completed, deserialized.completed);
        assert_eq!(summary.transient_failure, deserialized.transient_failure);
        assert_eq!(summary.permanent_failure, deserialized.permanent_failure);
    }
}
