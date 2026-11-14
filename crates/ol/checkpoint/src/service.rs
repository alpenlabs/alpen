//! Service framework integration for OL checkpoint builder.

use std::marker::PhantomData;

use serde::Serialize;
use strata_identifiers::Epoch;
use strata_service::{Response, Service, SyncService};
use strata_status::OLSyncStatusUpdate;
use tracing::{debug, error};

use crate::context::CheckpointWorkerContext;
use crate::errors::CheckpointNotReady;
use crate::state::OLCheckpointServiceState;

/// OL checkpoint builder service implementation.
///
/// Generic over the context type to support different storage/provider implementations.
#[derive(Debug)]
pub(crate) struct OLCheckpointService<C: CheckpointWorkerContext>(PhantomData<C>);

impl<C: CheckpointWorkerContext> Service for OLCheckpointService<C> {
    type State = OLCheckpointServiceState<C>;

    /// Wake-up after canonical index publication; status fields are not used for selection.
    type Msg = Option<OLSyncStatusUpdate>;

    type Status = OLCheckpointStatus;

    fn get_status(state: &Self::State) -> Self::Status {
        OLCheckpointStatus {
            is_initialized: state.is_initialized(),
            last_processed_epoch: state.last_processed_epoch(),
        }
    }
}

impl<C: CheckpointWorkerContext> SyncService for OLCheckpointService<C> {
    fn on_launch(state: &mut Self::State) -> anyhow::Result<()> {
        state.initialize()?;
        // Catch up even if no new fork-choice notification arrives after restart.
        process_canonical_chain(state)?;
        Ok(())
    }

    fn process_input(state: &mut Self::State, input: Self::Msg) -> anyhow::Result<Response> {
        if input.is_some() {
            process_canonical_chain(state)?;
        }
        Ok(Response::Continue)
    }
}

fn process_canonical_chain<C: CheckpointWorkerContext>(
    state: &mut OLCheckpointServiceState<C>,
) -> anyhow::Result<()> {
    if let Err(err) = state.handle_chain_update() {
        match err.downcast_ref::<CheckpointNotReady>() {
            Some(not_ready) => debug!(%not_ready, "checkpoint data not yet available"),
            None => {
                error!(%err, "checkpoint build failed");
                return Err(err);
            }
        }
    }
    Ok(())
}

/// Status information for the OL checkpoint builder service.
#[derive(Clone, Debug, Serialize)]
pub struct OLCheckpointStatus {
    /// Whether the worker has been initialized.
    pub is_initialized: bool,

    /// Last epoch processed, if any.
    pub last_processed_epoch: Option<Epoch>,
}
