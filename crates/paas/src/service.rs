//! ProverService implementation using AsyncService pattern

use std::sync::Arc;

use tokio::spawn;
use tracing::{debug, info};

use strata_service::{AsyncService, Response, Service};

use crate::commands::ProverCommand;
use crate::state::{ProverServiceState, StatusSummary};
use crate::worker::WorkerPool;
use crate::Prover;

/// Prover service that manages proof generation tasks
pub struct ProverService<P: Prover> {
    _phantom: std::marker::PhantomData<P>,
}

impl<P: Prover> Service for ProverService<P> {
    type State = ProverServiceState<P>;
    type Msg = ProverCommand<P::TaskId>;
    type Status = ProverServiceStatus;

    fn get_status(state: &Self::State) -> Self::Status {
        let summary = state.generate_summary();
        ProverServiceStatus { summary }
    }
}

impl<P: Prover> AsyncService for ProverService<P> {
    async fn on_launch(state: &mut Self::State) -> anyhow::Result<()> {
        info!("ProverService launching");

        // Spawn worker pool
        let worker_pool = WorkerPool::new(Arc::new(state.clone()));
        spawn(async move {
            worker_pool.run().await;
        });

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
                    Ok(()) => completion.send(()).await,
                    Err(e) => {
                        debug!(?task_id, ?e, "Failed to submit task");
                        // If task already exists, treat as success (idempotent operation)
                        if e.to_string().contains("Task already exists") {
                            completion.send(()).await;
                        }
                        // Other errors are logged but don't stop the service
                    }
                }
            }
            ProverCommand::GetStatus {
                task_id,
                completion,
            } => {
                debug!(?task_id, "Processing GetStatus command");
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
impl<P: Prover> Clone for ProverServiceState<P> {
    fn clone(&self) -> Self {
        Self {
            prover: self.prover.clone(),
            config: self.config.clone(),
            tasks: self.tasks.clone(),
            in_progress: self.in_progress.clone(),
        }
    }
}
