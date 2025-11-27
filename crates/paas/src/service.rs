//! ProverService implementation using AsyncService pattern

use strata_service::{AsyncService, Response, Service};
use tracing::{debug, info};

use crate::{
    commands::ProverCommand,
    state::{ProverServiceState, StatusSummary},
    task::TaskId,
    ProgramType,
};

/// Prover service that manages proof generation tasks
pub struct ProverService<P: ProgramType> {
    _phantom: std::marker::PhantomData<P>,
}

impl<P: ProgramType> Service for ProverService<P> {
    type State = ProverServiceState<P>;
    type Msg = ProverCommand<TaskId<P>>;
    type Status = ProverServiceStatus;

    fn get_status(state: &Self::State) -> Self::Status {
        let summary = state.generate_summary();
        ProverServiceStatus { summary }
    }
}

impl<P: ProgramType> AsyncService for ProverService<P> {
    async fn on_launch(_state: &mut Self::State) -> anyhow::Result<()> {
        info!("ProverService launching with channel-based workers");

        // Note: Worker pool is now spawned directly by the builder
        // before service launch, using the channel receiver

        info!("ProverService launched successfully");
        Ok(())
    }

    async fn process_input(state: &mut Self::State, input: &Self::Msg) -> anyhow::Result<Response> {
        match input {
            ProverCommand::SubmitTask {
                task_id,
                completion,
            } => {
                debug!(?task_id, "Processing SubmitTask command");
                match state.submit_task(task_id.clone()) {
                    Ok((uuid, is_new)) => {
                        if is_new {
                            debug!(?task_id, %uuid, "New task submitted with UUID");

                            // Spawn task execution only for new tasks (avoid double execution)
                            let state_clone = state.clone();
                            let task_id_clone = task_id.clone();
                            state.executor.handle().spawn(async move {
                                state_clone.execute_and_track(task_id_clone).await;
                            });
                        } else {
                            debug!(?task_id, %uuid, "Task already exists, returning existing UUID");
                        }

                        completion.send(uuid).await;
                    }
                    Err(e) => {
                        debug!(?task_id, ?e, "Failed to submit task");
                        // Task store error (e.g., DB error) - don't send response
                        // Completion channel will be dropped, signaling error to caller
                    }
                }
            }
            ProverCommand::ExecuteTask {
                task_id,
                completion,
            } => {
                debug!(?task_id, "Processing ExecuteTask command (awaitable)");
                // Execute synchronously and await result
                let result = state.execute_task_sync(task_id.clone()).await;
                debug!(?task_id, ?result, "ExecuteTask completed");
                completion.send(result).await;
            }
            ProverCommand::GetStatusByUuid { uuid, completion } => {
                debug!(%uuid, "Processing GetStatusByUuid command");
                let result = state.get_status_by_uuid(uuid).ok();
                if let Some(status) = result {
                    completion.send(status).await;
                }
            }
            ProverCommand::GetStatusByTaskId {
                task_id,
                completion,
            } => {
                debug!(?task_id, "Processing GetStatusByTaskId command (internal)");
                let result = state.get_status(task_id).ok();
                if let Some(status) = result {
                    completion.send(status).await;
                }
            }
            ProverCommand::GetSummary { completion } => {
                debug!("Processing GetSummary command");
                let summary = state.generate_summary();
                completion.send(summary).await;
            }
        }

        Ok(Response::Continue)
    }

    async fn before_shutdown(
        _state: &mut Self::State,
        _err: Option<&anyhow::Error>,
    ) -> anyhow::Result<()> {
        info!("ProverService shutting down");
        // Worker pool tasks will be cancelled automatically
        Ok(())
    }
}

/// Service status for monitoring (internal)
#[derive(Clone, Debug, serde::Serialize)]
pub struct ProverServiceStatus {
    pub(crate) summary: StatusSummary,
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
            timer: self.timer.clone(),
        }
    }
}
