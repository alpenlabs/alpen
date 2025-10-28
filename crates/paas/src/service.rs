//! Prover service implementation

use strata_db::traits::ProofDatabase;
use strata_service::{AsyncService, Response, Service};
use tracing::{debug, error, info};

use crate::{state::ProverServiceState, PaaSCommand, PaaSStatus};

/// Prover service implementation following AsyncService pattern
pub struct ProverService<D: ProofDatabase> {
    _phantom: std::marker::PhantomData<D>,
}

impl<D: ProofDatabase> Service for ProverService<D> {
    type State = ProverServiceState<D>;
    type Msg = PaaSCommand;
    type Status = PaaSStatus;

    fn get_status(s: &Self::State) -> Self::Status {
        s.get_status()
    }
}

impl<D: ProofDatabase> AsyncService for ProverService<D> {
    async fn on_launch(state: &mut Self::State) -> anyhow::Result<()> {
        info!("PaaS service launched");
        state.update_status();
        Ok(())
    }

    async fn process_input(state: &mut Self::State, input: &Self::Msg) -> anyhow::Result<Response> {
        debug!("Processing PaaS command");

        match input {
            PaaSCommand::CreateTask {
                context,
                deps,
                completion,
            } => {
                let result = state.create_task(*context, deps.to_vec());
                if let Ok(task_id) = &result {
                    info!(?task_id, "Created proof task");
                }
                completion.send(result).await;
                Ok(Response::Continue)
            }

            PaaSCommand::GetTaskStatus {
                task_id,
                completion,
            } => {
                let result = state.get_task_status(*task_id);
                completion.send(result).await;
                Ok(Response::Continue)
            }

            PaaSCommand::GetProof {
                task_id,
                completion,
            } => {
                let result = state.get_proof(*task_id);
                if let Ok(Some(_)) = &result {
                    debug!(?task_id, "Retrieved proof for task");
                }
                completion.send(result).await;
                Ok(Response::Continue)
            }

            PaaSCommand::CancelTask {
                task_id,
                completion,
            } => {
                let result = state.cancel_task(*task_id);
                if result.is_ok() {
                    info!(?task_id, "Cancelled task");
                }
                completion.send(result).await;
                Ok(Response::Continue)
            }

            PaaSCommand::GetReport { completion } => {
                let report = state.generate_report();
                completion.send(Ok(report)).await;
                Ok(Response::Continue)
            }
        }
    }

    async fn before_shutdown(
        _state: &mut Self::State,
        err: Option<&anyhow::Error>,
    ) -> anyhow::Result<()> {
        if let Some(e) = err {
            error!("PaaS service shutting down due to error: {}", e);
        } else {
            info!("PaaS service shutting down gracefully");
        }
        Ok(())
    }
}
