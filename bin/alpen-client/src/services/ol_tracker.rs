use std::{sync::Arc, time::Duration};

use alpen_ee_common::{ConsensusHeads, OLClient, OLFinalizedStatus, Storage};
use alpen_ee_ol_tracker::{
    OLTrackerService, OLTrackerServiceState, OLTrackerState, OLTrackerStatus,
};
use strata_service::{
    AsyncExecutor, DumbTickHandle, DumbTickingInput, ServiceBuilder, ServiceMonitor,
};
use tokio::sync::watch;

/// Default number of OL epochs to process in each cycle.
const DEFAULT_MAX_EPOCHS_FETCH: u32 = 10;
/// Default ms to wait between OL polls.
const DEFAULT_POLL_WAIT_MS: u64 = 1_000;

/// Handle for accessing OL tracker state updates.
#[derive(Debug)]
pub(crate) struct OLTrackerHandle {
    ol_status_rx: watch::Receiver<OLFinalizedStatus>,
    consensus_rx: watch::Receiver<ConsensusHeads>,
    monitor: ServiceMonitor<OLTrackerStatus>,
    tick_handle: DumbTickHandle,
}

impl OLTrackerHandle {
    /// Returns a watcher for OL finalized status updates.
    pub(crate) fn ol_status_watcher(&self) -> watch::Receiver<OLFinalizedStatus> {
        self.ol_status_rx.clone()
    }

    /// Returns a watcher for consensus head updates.
    pub(crate) fn consensus_watcher(&self) -> watch::Receiver<ConsensusHeads> {
        self.consensus_rx.clone()
    }

    /// Returns the service framework monitor.
    #[expect(dead_code, reason = "part of handle API, not yet used")]
    pub(crate) fn monitor(&self) -> &ServiceMonitor<OLTrackerStatus> {
        &self.monitor
    }

    /// Stops the OL tracker service.
    ///
    /// Returns false if the service was already stopping.
    #[expect(dead_code, reason = "part of handle API, not yet used")]
    pub(crate) fn stop(self) -> bool {
        self.tick_handle.stop()
    }
}

/// Starts the OL tracker as a service framework service.
///
/// Uses default polling interval and max epochs per cycle.
pub(crate) async fn start_ol_tracker_service<TStorage, TOLClient>(
    state: OLTrackerState,
    genesis_epoch: u32,
    storage: Arc<TStorage>,
    ol_client: Arc<TOLClient>,
    executor: &impl AsyncExecutor,
) -> anyhow::Result<OLTrackerHandle>
where
    TStorage: Storage + 'static,
    TOLClient: OLClient + 'static,
{
    start_ol_tracker_service_with(
        state,
        genesis_epoch,
        storage,
        ol_client,
        DEFAULT_POLL_WAIT_MS,
        DEFAULT_MAX_EPOCHS_FETCH,
        executor,
    )
    .await
}

/// Starts the OL tracker as a service with custom configuration.
pub(crate) async fn start_ol_tracker_service_with<TStorage, TOLClient>(
    state: OLTrackerState,
    genesis_epoch: u32,
    storage: Arc<TStorage>,
    ol_client: Arc<TOLClient>,
    poll_wait_ms: u64,
    max_epochs_fetch: u32,
    executor: &impl AsyncExecutor,
) -> anyhow::Result<OLTrackerHandle>
where
    TStorage: Storage + 'static,
    TOLClient: OLClient + 'static,
{
    // Initialize watch channels from current state.
    let (ol_status_tx, ol_status_rx) = watch::channel(state.get_ol_status());
    let (consensus_tx, consensus_rx) = watch::channel(state.get_consensus_heads());

    let service_state = OLTrackerServiceState {
        storage,
        ol_client,
        genesis_epoch,
        max_epochs_fetch,
        ol_status_tx,
        consensus_tx,
        tracker_state: state,
    };

    let (tick_handle, tick_input) = DumbTickingInput::new(Duration::from_millis(poll_wait_ms));

    let monitor = ServiceBuilder::<OLTrackerService<_, _>, _>::new()
        .with_state(service_state)
        .with_input(tick_input)
        .launch_async("ol_tracker", executor)
        .await?;

    Ok(OLTrackerHandle {
        ol_status_rx,
        consensus_rx,
        monitor,
        tick_handle,
    })
}
