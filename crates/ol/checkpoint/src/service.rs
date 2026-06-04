//! Service framework integration for OL checkpoint builder.

use std::marker::PhantomData;

use serde::Serialize;
use strata_identifiers::Epoch;
use strata_primitives::epoch::EpochCommitment;
use strata_service::{Response, Service, SyncService};
use tracing::{error, warn};

use crate::{
    context::CheckpointWorkerContext, errors::CheckpointNotReady, state::OLCheckpointServiceState,
};

/// OL checkpoint builder service implementation.
///
/// Generic over the context type to support different storage/provider implementations.
#[derive(Debug)]
pub(crate) struct OLCheckpointService<C: CheckpointWorkerContext>(PhantomData<C>);

impl<C: CheckpointWorkerContext> Service for OLCheckpointService<C> {
    type State = OLCheckpointServiceState<C>;

    /// Input from a [`watch::Receiver<Option<EpochCommitment>>`].
    ///
    /// `None` = initial state (no epoch completed yet), skip processing.
    /// `Some(commitment)` = an epoch was completed, process checkpoint.
    type Msg = Option<EpochCommitment>;

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
        state.initialize();

        // Startup catch-up: build any checkpoint payloads for epochs that were
        // summarized before a restart but never checkpointed. The checkpoint
        // worker is otherwise only driven by the epoch-summary watch channel,
        // whose next message arrives only when a new epoch completes (gated on a
        // new L1 block). Without this, a restart would stall checkpoint signing
        // until the next L1 block was mined.
        if let Err(err) = state.catch_up_to_latest_summary() {
            match err.downcast_ref::<CheckpointNotReady>() {
                Some(not_ready) => {
                    warn!(%not_ready, "startup checkpoint catch-up deferred; epoch data not yet available");
                }
                None => {
                    error!(%err, "startup checkpoint catch-up failed");
                    return Err(err);
                }
            }
        }

        Ok(())
    }

    fn process_input(state: &mut Self::State, input: Self::Msg) -> anyhow::Result<Response> {
        // Skip if no epoch commitment yet (initial watch channel state)
        let Some(commitment) = input else {
            return Ok(Response::Continue);
        };

        let epoch = commitment.epoch();
        if let Err(err) = state.handle_complete_epoch(commitment) {
            match err.downcast_ref::<CheckpointNotReady>() {
                Some(not_ready) => {
                    warn!(%epoch, %not_ready, "epoch data not yet available");
                }
                None => {
                    error!(%epoch, %err, "checkpoint build failed");
                    return Err(err);
                }
            }
        }
        Ok(Response::Continue)
    }
}

/// Status information for the OL checkpoint builder service.
#[derive(Clone, Debug, Serialize)]
pub struct OLCheckpointStatus {
    /// Whether the worker has been initialized.
    pub is_initialized: bool,

    /// Last epoch processed, if any.
    pub last_processed_epoch: Option<Epoch>,
}
