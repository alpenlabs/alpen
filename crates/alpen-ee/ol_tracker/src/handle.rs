use std::{future::Future, sync::Arc};

use alpen_ee_common::{ConsensusHeads, OLClient, Storage};
use alpen_ee_config::AlpenEeParams;
use strata_ee_acct_types::EeAccountState;
use tokio::sync::watch;

use crate::{ctx::OLTrackerCtx, state::OLTrackerState, task::ol_tracker_task};

/// Default number of OL blocks to process in each cycle
const DEFAULT_MAX_BLOCKS_FETCH: u64 = 10;
/// Default ms to wait between ol polls
const DEFAULT_POLL_WAIT_MS: u64 = 100;
/// Default number of OL blocks to process in each cycle during reorg
const DEFAULT_REORG_FETCH_SIZE: u64 = 10;

/// Handle for accessing OL tracker state updates.
#[derive(Debug)]
pub struct OLTrackerHandle {
    ee_state_rx: watch::Receiver<EeAccountState>,
    consensus_rx: watch::Receiver<ConsensusHeads>,
}

impl OLTrackerHandle {
    /// Returns a watcher for EE account state updates.
    pub fn ee_state_watcher(&self) -> watch::Receiver<EeAccountState> {
        self.ee_state_rx.clone()
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
    params: Arc<AlpenEeParams>,
    storage: Arc<TStorage>,
    ol_client: Arc<TOLClient>,
    max_block_fetch: Option<u64>,
    poll_wait_ms: Option<u64>,
    reorg_fetch_size: Option<u64>,
}

impl<TStorage, TOLClient> OLTrackerBuilder<TStorage, TOLClient> {
    /// Creates a new OL tracker builder with all required fields.
    pub fn new(
        state: OLTrackerState,
        params: Arc<AlpenEeParams>,
        storage: Arc<TStorage>,
        ol_client: Arc<TOLClient>,
    ) -> Self {
        Self {
            state,
            params,
            storage,
            ol_client,
            max_block_fetch: None,
            poll_wait_ms: None,
            reorg_fetch_size: None,
        }
    }

    /// Sets the maximum number of blocks to fetch per cycle.
    pub fn with_max_block_fetch(mut self, v: u64) -> Self {
        self.max_block_fetch = Some(v);
        self
    }

    /// Sets the polling wait time in milliseconds.
    pub fn with_poll_wait_ms(mut self, v: u64) -> Self {
        self.poll_wait_ms = Some(v);
        self
    }

    /// Sets the reorg fetch size for handling reorganizations.
    pub fn with_reorg_fetch_size(mut self, v: u64) -> Self {
        self.reorg_fetch_size = Some(v);
        self
    }

    /// Builds and returns the tracker handle and task.
    pub fn build(self) -> (OLTrackerHandle, impl Future<Output = ()>)
    where
        TStorage: Storage,
        TOLClient: OLClient,
    {
        let (ee_state_tx, ee_state_rx) = watch::channel(self.state.best_ee_state().clone());
        let (consensus_tx, consensus_rx) = watch::channel(self.state.get_consensus_heads());
        let handle = OLTrackerHandle {
            ee_state_rx,
            consensus_rx,
        };
        let ctx = OLTrackerCtx {
            storage: self.storage,
            params: self.params,
            ol_client: self.ol_client,
            ee_state_tx,
            consensus_tx,
            max_blocks_fetch: self.max_block_fetch.unwrap_or(DEFAULT_MAX_BLOCKS_FETCH),
            poll_wait_ms: self.poll_wait_ms.unwrap_or(DEFAULT_POLL_WAIT_MS),
            reorg_fetch_size: self.reorg_fetch_size.unwrap_or(DEFAULT_REORG_FETCH_SIZE),
        };
        let task = ol_tracker_task(self.state, ctx);

        (handle, task)
    }
}
