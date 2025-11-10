use std::{future::Future, sync::Arc};

use alpen_ee_common::{ConsensusHeads, OlClient, Storage};
use alpen_ee_config::AlpenEeParams;
use strata_ee_acct_types::EeAccountState;
use tokio::sync::watch;

use crate::{ctx::OlTrackerCtx, state::OlTrackerState, task::ol_tracker_task};

/// Default number of Ol blocks to process in each cycle
const DEFAULT_MAX_BLOCKS_FETCH: u64 = 10;
/// Default ms to wait between ol polls
const DEFAULT_POLL_WAIT_MS: u64 = 100;
/// Default number of Ol blocks to process in each cycle during reorg
const DEFAULT_REORG_FETCH_SIZE: u64 = 10;

#[derive(Debug)]
pub struct OlTrackerHandle {
    ee_state_rx: watch::Receiver<EeAccountState>,
    consensus_rx: watch::Receiver<ConsensusHeads>,
}

impl OlTrackerHandle {
    pub fn ee_state_watcher(&self) -> watch::Receiver<EeAccountState> {
        self.ee_state_rx.clone()
    }

    pub fn consensus_watcher(&self) -> watch::Receiver<ConsensusHeads> {
        self.consensus_rx.clone()
    }
}

#[derive(Debug)]
pub struct OlTrackerBuilder<TStorage, TOlClient> {
    state: OlTrackerState,
    params: Arc<AlpenEeParams>,
    storage: Arc<TStorage>,
    ol_client: Arc<TOlClient>,
    max_block_fetch: Option<u64>,
    poll_wait_ms: Option<u64>,
    reorg_fetch_size: Option<u64>,
}

#[allow(
    dead_code,
    clippy::allow_attributes,
    reason = "optional builder methods"
)]
impl<TStorage, TOlClient> OlTrackerBuilder<TStorage, TOlClient> {
    pub fn new(
        state: OlTrackerState,
        params: Arc<AlpenEeParams>,
        storage: Arc<TStorage>,
        ol_client: Arc<TOlClient>,
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

    pub fn with_max_block_fetch(mut self, v: u64) -> Self {
        self.max_block_fetch = Some(v);
        self
    }

    pub fn with_poll_wait_ms(mut self, v: u64) -> Self {
        self.poll_wait_ms = Some(v);
        self
    }

    pub fn with_reorg_fetch_size(mut self, v: u64) -> Self {
        self.reorg_fetch_size = Some(v);
        self
    }

    pub fn build(self) -> (OlTrackerHandle, impl Future<Output = ()>)
    where
        TStorage: Storage,
        TOlClient: OlClient,
    {
        let (ee_state_tx, ee_state_rx) = watch::channel(self.state.best_ee_state().clone());
        let (consensus_tx, consensus_rx) = watch::channel(self.state.get_consensus_heads());
        let handle = OlTrackerHandle {
            ee_state_rx,
            consensus_rx,
        };
        let ctx = OlTrackerCtx {
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
