//! Service framework integration for the OL tracker.

use std::{marker::PhantomData, sync::Arc};

use alpen_ee_common::{ConsensusHeads, OLClient, OLFinalizedStatus, Storage};
use serde::Serialize;
use strata_service::{AsyncService, Response, Service, ServiceState};
use tokio::sync::watch;
use tracing::{debug, error};

use crate::{
    error::OLTrackerError,
    reorg::handle_reorg,
    state::OLTrackerState,
    task::{handle_extend_ee_state, track_ol_state, TrackOLAction},
};

/// OL tracker service marker type.
#[derive(Debug)]
pub struct OLTrackerService<TStorage, TOLClient>(PhantomData<(TStorage, TOLClient)>);

/// Minimal status for the service framework.
///
/// The actual useful status is communicated via dedicated watch channels
/// held in [`OLTrackerServiceState`] for backward compatibility with
/// downstream consumers.
#[derive(Clone, Debug, Default, Serialize)]
pub struct OLTrackerStatus;

/// Service state for the OL tracker, combining dependencies and mutable tracking state.
pub struct OLTrackerServiceState<TStorage, TOLClient> {
    pub storage: Arc<TStorage>,
    pub ol_client: Arc<TOLClient>,
    pub genesis_epoch: u32,
    pub max_epochs_fetch: u32,
    pub ol_status_tx: watch::Sender<OLFinalizedStatus>,
    pub consensus_tx: watch::Sender<ConsensusHeads>,
    pub tracker_state: OLTrackerState,
}

impl<TStorage, TOLClient> std::fmt::Debug for OLTrackerServiceState<TStorage, TOLClient> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OLTrackerServiceState")
            .field("genesis_epoch", &self.genesis_epoch)
            .field("max_epochs_fetch", &self.max_epochs_fetch)
            .finish_non_exhaustive()
    }
}

impl<TStorage, TOLClient> OLTrackerServiceState<TStorage, TOLClient> {
    /// Notifies watchers of the latest OL status and consensus heads.
    pub fn notify_watchers(&self) {
        let _ = self.ol_status_tx.send(self.tracker_state.get_ol_status());
        let _ = self
            .consensus_tx
            .send(self.tracker_state.get_consensus_heads());
    }
}

impl<TStorage, TOLClient> ServiceState for OLTrackerServiceState<TStorage, TOLClient>
where
    TStorage: Storage + 'static,
    TOLClient: OLClient + 'static,
{
    fn name(&self) -> &str {
        "ol_tracker"
    }

    fn span_prefix(&self) -> &str {
        "ol_tracker"
    }
}

impl<TStorage, TOLClient> Service for OLTrackerService<TStorage, TOLClient>
where
    TStorage: Storage + 'static,
    TOLClient: OLClient + 'static,
{
    type State = OLTrackerServiceState<TStorage, TOLClient>;
    type Msg = ();
    type Status = OLTrackerStatus;

    fn get_status(_state: &Self::State) -> Self::Status {
        OLTrackerStatus
    }
}

impl<TStorage, TOLClient> AsyncService for OLTrackerService<TStorage, TOLClient>
where
    TStorage: Storage + 'static,
    TOLClient: OLClient + 'static,
{
    async fn process_input(state: &mut Self::State, _input: ()) -> anyhow::Result<Response> {
        match track_ol_state(
            &state.tracker_state,
            state.ol_client.as_ref(),
            state.max_epochs_fetch,
        )
        .await
        {
            Ok(TrackOLAction::Extend(epoch_operations, chain_status)) => {
                debug!(?epoch_operations, ?chain_status, "received track action");
                if let Err(error) = handle_extend_ee_state(
                    &epoch_operations,
                    &chain_status,
                    &mut state.tracker_state,
                    state.storage.as_ref(),
                )
                .await
                {
                    return handle_tracker_error(error);
                }
                state.notify_watchers();
            }
            Ok(TrackOLAction::Reorg) => {
                debug!("received reorg action");
                if let Err(error) = handle_reorg(
                    &mut state.tracker_state,
                    state.storage.as_ref(),
                    state.ol_client.as_ref(),
                    state.genesis_epoch,
                )
                .await
                {
                    return handle_tracker_error(error);
                }
                state.notify_watchers();
            }
            Ok(TrackOLAction::Noop) => {
                debug!("received noop action");
            }
            Err(error) => {
                return handle_tracker_error(error);
            }
        }

        Ok(Response::Continue)
    }
}

/// Handles OL tracker errors within the service framework.
///
/// Fatal errors return `Err` to stop the service (task executor will panic
/// on critical task failure). Recoverable errors are logged and return
/// `Ok(Continue)` to retry on the next tick.
fn handle_tracker_error(error: impl Into<OLTrackerError>) -> anyhow::Result<Response> {
    let error = error.into();

    if error.is_fatal() {
        Err(anyhow::anyhow!("{}", error.panic_message()))
    } else {
        error!(%error, "recoverable error in ol tracker");
        Ok(Response::Continue)
    }
}
