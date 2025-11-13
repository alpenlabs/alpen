//! Worker pool implementation

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::{spawn, time::sleep};
use tracing::{debug, error, info};

use crate::error::PaaSError;
use crate::state::ProverServiceState;
use crate::task::TaskStatus;
use crate::Prover;

/// Worker pool that processes tasks for all backends
pub struct WorkerPool<P: Prover> {
    state: Arc<ProverServiceState<P>>,
}

impl<P: Prover> WorkerPool<P> {
    /// Create a new worker pool
    pub fn new(state: Arc<ProverServiceState<P>>) -> Self {
        Self { state }
    }

    /// Main worker pool loop
    pub async fn run(self) {
        info!("Worker pool started");

        loop {
            // Get pending tasks
            let pending = self.state.list_pending();

            // Get retriable tasks
            let retriable = self.state.list_retriable();

            // Process both pending and retriable tasks
            for task_id in pending.into_iter().chain(retriable) {
                self.try_spawn_task(task_id).await;
            }

            // Sleep before next poll
            sleep(Duration::from_millis(
                self.state.config().workers.polling_interval_ms,
            ))
            .await;
        }
    }

    /// Try to spawn a task if worker capacity allows
    async fn try_spawn_task(&self, task_id: P::TaskId) {
        let backend = self.state.prover().backend(&task_id);

        // Check worker limits
        let worker_limit = self
            .state
            .config()
            .workers
            .worker_count
            .get(&backend)
            .copied()
            .unwrap_or(0);

        // Atomically check and increment counter
        let should_skip = {
            let mut in_progress = self.state.in_progress_counter().lock().unwrap();
            let current = in_progress.get(&backend).copied().unwrap_or(0);

            if current >= worker_limit {
                debug!(?backend, current, worker_limit, "Worker limit reached, skipping task");
                true
            } else {
                *in_progress.entry(backend.clone()).or_insert(0) += 1;
                debug!(?backend, old=current, new=current+1, "Incremented worker counter");
                false
            }
        };

        if should_skip {
            return;
        }

        // Spawn proof generation task
        let state = self.state.clone();
        let backend_for_guard = backend.clone();

        spawn(async move {
            info!(?task_id, "Starting proof generation");

            // RAII guard ensures counter is decremented on drop
            let _guard = WorkerGuard::new(state.in_progress_counter().clone(), backend_for_guard);

            // Transition: Pending → Queued → Proving
            if let Err(e) = state.update_status(&task_id, TaskStatus::Queued) {
                error!(?task_id, ?e, "Failed to mark task as queued");
                return;
            }

            if let Err(e) = state.update_status(&task_id, TaskStatus::Proving) {
                error!(?task_id, ?e, "Failed to mark task as proving");
                return;
            }

            // Call the prover
            let result = state.prover().prove(task_id.clone()).await;

            // Handle result
            match result {
                Ok(()) => {
                    info!(?task_id, "Proof generation completed");
                    if let Err(e) = state.update_status(&task_id, TaskStatus::Completed) {
                        error!(?task_id, ?e, "Failed to mark task as completed");
                    }
                }
                Err(PaaSError::TransientFailure(msg)) => {
                    error!(?task_id, error=%msg, "Transient failure");

                    // Get current retry count
                    let retry_count = if let Ok(TaskStatus::TransientFailure { retry_count, .. }) = state.get_status(&task_id) {
                        retry_count + 1
                    } else {
                        1
                    };

                    // Check if should retry
                    if state.config().retry.should_retry(retry_count) {
                        if let Err(e) = state.update_status(
                            &task_id,
                            TaskStatus::TransientFailure {
                                retry_count,
                                error: msg,
                            },
                        ) {
                            error!(?task_id, ?e, "Failed to mark task as transient failure");
                        }
                    } else {
                        // Max retries exceeded, mark as permanent failure
                        if let Err(e) = state.update_status(
                            &task_id,
                            TaskStatus::PermanentFailure { error: msg },
                        ) {
                            error!(?task_id, ?e, "Failed to mark task as permanent failure");
                        }
                    }
                }
                Err(PaaSError::PermanentFailure(msg)) => {
                    error!(?task_id, error=%msg, "Permanent failure");
                    if let Err(e) = state.update_status(
                        &task_id,
                        TaskStatus::PermanentFailure { error: msg },
                    ) {
                        error!(?task_id, ?e, "Failed to mark task as permanent failure");
                    }
                }
                Err(e) => {
                    // Treat other errors as transient
                    error!(?task_id, ?e, "Unexpected error, treating as transient");
                    if let Err(e) = state.update_status(
                        &task_id,
                        TaskStatus::TransientFailure {
                            retry_count: 1,
                            error: e.to_string(),
                        },
                    ) {
                        error!(?task_id, ?e, "Failed to update status");
                    }
                }
            }
        });
    }
}

/// RAII guard that decrements the worker counter when dropped
struct WorkerGuard<B: Clone + Eq + std::hash::Hash + std::fmt::Debug> {
    counter: Arc<Mutex<HashMap<B, usize>>>,
    backend: B,
}

impl<B: Clone + Eq + std::hash::Hash + std::fmt::Debug> WorkerGuard<B> {
    fn new(counter: Arc<Mutex<HashMap<B, usize>>>, backend: B) -> Self {
        Self { counter, backend }
    }
}

impl<B: Clone + Eq + std::hash::Hash + std::fmt::Debug> Drop for WorkerGuard<B> {
    fn drop(&mut self) {
        let mut counter = self.counter.lock().unwrap();
        if let Some(count) = counter.get_mut(&self.backend) {
            let old = *count;
            *count = count.saturating_sub(1);
            debug!(backend=?self.backend, old, new=*count, "Worker guard dropped, decremented counter");
        }
    }
}
