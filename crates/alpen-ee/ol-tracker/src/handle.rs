use std::{future::Future, sync::Arc};

use alpen_ee_common::{ConsensusHeads, OLClient, OLFinalizedStatus, Storage};
use tokio::sync::watch;

use crate::{ctx::OLTrackerCtx, state::OLTrackerState, task::ol_tracker_task};

/// Default number of OL epochs to process in each cycle
const DEFAULT_MAX_EPOCHS_FETCH: u32 = 10;
/// Default ms to wait between ol polls
const DEFAULT_POLL_WAIT_MS: u64 = 1_000;

/// Handle for accessing OL tracker state updates.
#[derive(Debug)]
pub struct OLTrackerHandle {
    ol_status_rx: watch::Receiver<OLFinalizedStatus>,
    consensus_rx: watch::Receiver<ConsensusHeads>,
}

impl OLTrackerHandle {
    /// Returns a watcher for EE account state updates.
    pub fn ol_status_watcher(&self) -> watch::Receiver<OLFinalizedStatus> {
        self.ol_status_rx.clone()
    }

    /// Returns a watcher for consensus head updates.
    pub fn consensus_watcher(&self) -> watch::Receiver<ConsensusHeads> {
        self.consensus_rx.clone()
    }
}

/// Builder for creating an OL tracker with custom configuration.
#[derive(Debug)]
pub struct OLTrackerBuilder<TStorage, TOLClient> {
    state: OLTrackerState,
    genesis_epoch: u32,
    storage: Arc<TStorage>,
    ol_client: Arc<TOLClient>,
    max_epochs_fetch: Option<u32>,
    poll_wait_ms: Option<u64>,
}

impl<TStorage, TOLClient> OLTrackerBuilder<TStorage, TOLClient> {
    /// Creates a new OL tracker builder with all required fields.
    pub fn new(
        state: OLTrackerState,
        genesis_epoch: u32,
        storage: Arc<TStorage>,
        ol_client: Arc<TOLClient>,
    ) -> Self {
        Self {
            state,
            genesis_epoch,
            storage,
            ol_client,
            max_epochs_fetch: None,
            poll_wait_ms: None,
        }
    }

    /// Sets the maximum number of epochs to fetch per cycle.
    pub fn with_max_epochs_fetch(mut self, v: u32) -> Self {
        self.max_epochs_fetch = Some(v);
        self
    }

    /// Sets the polling wait time in milliseconds.
    pub fn with_poll_wait_ms(mut self, v: u64) -> Self {
        self.poll_wait_ms = Some(v);
        self
    }

    /// Builds and returns the tracker handle and task.
    pub fn build(self) -> (OLTrackerHandle, impl Future<Output = ()>)
    where
        TStorage: Storage,
        TOLClient: OLClient,
    {
        let (ol_status_tx, ol_status_rx) = watch::channel(self.state.get_ol_status());
        let (consensus_tx, consensus_rx) = watch::channel(self.state.get_consensus_heads());
        let handle = OLTrackerHandle {
            ol_status_rx,
            consensus_rx,
        };
        let ctx = OLTrackerCtx {
            storage: self.storage,
            ol_client: self.ol_client,
            genesis_epoch: self.genesis_epoch,
            ol_status_tx,
            consensus_tx,
            max_epochs_fetch: self.max_epochs_fetch.unwrap_or(DEFAULT_MAX_EPOCHS_FETCH),
            poll_wait_ms: self.poll_wait_ms.unwrap_or(DEFAULT_POLL_WAIT_MS),
        };
        let task = ol_tracker_task(self.state, ctx);

        (handle, task)
    }
}
