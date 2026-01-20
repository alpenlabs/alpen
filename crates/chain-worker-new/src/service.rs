//! Service framework integration for chain worker.

use std::{fmt::Debug, marker::PhantomData};

use serde::Serialize;
use strata_identifiers::OLBlockCommitment;
use strata_ol_state_types::OLState;
use strata_primitives::epoch::EpochCommitment;
use strata_service::{Response, Service, SyncService};

use crate::{
    message::ChainWorkerMessage, state::ChainWorkerServiceState, traits::ChainWorkerContext,
};

/// Chain worker service implementation using the service framework.
///
/// Generic over the state type for `StatusChannel`, defaulting to `OLState`.
#[derive(Debug)]
pub struct ChainWorkerService<
    W: ChainWorkerContext + Send + Sync + 'static,
    State: Clone + Debug + Send + Sync + 'static = OLState,
> {
    _phantom: PhantomData<(W, State)>,
}

impl<W: ChainWorkerContext + Send + Sync + 'static, State: Clone + Debug + Send + Sync + 'static>
    Service for ChainWorkerService<W, State>
{
    type State = ChainWorkerServiceState<W, State>;
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

impl<W: ChainWorkerContext + Send + Sync + 'static, State: Clone + Debug + Send + Sync + 'static>
    SyncService for ChainWorkerService<W, State>
{
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
