//! Service framework integration for chain worker.

use serde::Serialize;
use strata_identifiers::OLBlockCommitment;
use strata_ol_node::context::NodeContext;
use strata_primitives::epoch::EpochCommitment;
use strata_service::{Response, Service, ServiceBuilder, SyncService};

use crate::{
    ChainWorkerContextImpl, ChainWorkerHandle, WorkerError, message::ChainWorkerMessage,
    state::ChainWorkerServiceState,
};

/// Chain worker service implementation using the service framework.
#[derive(Debug)]
pub struct ChainWorkerService;

impl Service for ChainWorkerService {
    type State = ChainWorkerServiceState;
    type Msg = ChainWorkerMessage;
    type Status = ChainWorkerStatus;

    fn get_status(state: &Self::State) -> Self::Status {
        ChainWorkerStatus {
            is_initialized: state.is_initialized(),
            cur_tip: state.cur_tip(),
            last_finalized_epoch: state.last_finalized_epoch(),
        }
    }
}

impl SyncService for ChainWorkerService {
    fn on_launch(state: &mut Self::State) -> anyhow::Result<()> {
        let cur_tip = state.wait_for_genesis_and_resolve_tip()?;
        state.initialize_with_tip(cur_tip)?;
        Ok(())
    }

    fn process_input(state: &mut Self::State, input: &Self::Msg) -> anyhow::Result<Response> {
        match input {
            ChainWorkerMessage::TryExecBlock(olbc, completion) => {
                let res = state.try_exec_block(olbc);
                completion.send_blocking(res);
            }

            ChainWorkerMessage::UpdateSafeTip(olbc, completion) => {
                let res = state.update_cur_tip(*olbc);
                completion.send_blocking(res);
            }

            ChainWorkerMessage::FinalizeEpoch(epoch, completion) => {
                let res = state.finalize_epoch(*epoch);
                completion.send_blocking(res);
            }
        }

        Ok(Response::Continue)
    }
}

/// Status information for the chain worker service.
#[derive(Clone, Debug, Serialize)]
pub struct ChainWorkerStatus {
    /// Whether the worker has been initialized.
    pub is_initialized: bool,

    /// Current tip commitment.
    pub cur_tip: OLBlockCommitment,

    /// Last finalized epoch, if any.
    pub last_finalized_epoch: Option<EpochCommitment>,
}

/// Starts chain worker service from node ctx
pub fn start_chain_worker_service_from_ctx(
    nodectx: &NodeContext,
) -> anyhow::Result<ChainWorkerHandle> {
    let ctx = ChainWorkerContextImpl::from_node_context(nodectx);
    let state = ChainWorkerServiceState::new(ctx);
    let mut builder = ServiceBuilder::<ChainWorkerService, _>::new().with_state(state);

    // Create the command handle before launching.
    let command_handle = builder.create_command_handle(64);

    // Launch the service using the sync worker.
    let monitor = builder
        .launch_sync("chain_worker_new", nodectx.executor().as_ref())
        .map_err(|e| WorkerError::Unexpected(format!("failed to launch service: {}", e)))?;

    Ok(ChainWorkerHandle::new(command_handle, monitor))
}
