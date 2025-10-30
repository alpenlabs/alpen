use std::{future::Future, sync::Arc};

use strata_ee_acct_types::EeAccountState;
use tokio::sync::watch;

use crate::{
    ol_tracker::{
        state::ConsensusHeads,
        task::{ol_tracker_task, OlTrackerCtx, DEFAULT_MAX_BLOCKS_FETCH, DEFAULT_POLL_WAIT_MS},
        OlTrackerState,
    },
    traits::{ol_client::OlClient, storage::Storage},
};

pub(crate) struct OlTrackerHandle {
    ee_state_rx: watch::Receiver<EeAccountState>,
    consensus_rx: watch::Receiver<ConsensusHeads>,
}

impl OlTrackerHandle {
    pub(crate) fn create<TStorage, TOlClient>(
        state: OlTrackerState,
        storage: Arc<TStorage>,
        ol_client: Arc<TOlClient>,
        max_block_fetch: Option<u64>,
        poll_wait_ms: Option<u64>,
    ) -> (Self, impl Future<Output = ()>)
    where
        TStorage: Storage,
        TOlClient: OlClient,
    {
        let (ee_state_tx, ee_state_rx) = watch::channel(state.best_ee_state().clone());
        let (consensus_tx, consensus_rx) = watch::channel(state.get_consensus_heads());
        let handle = Self {
            ee_state_rx,
            consensus_rx,
        };
        let ctx = OlTrackerCtx {
            storage,
            ol_client,
            ee_state_tx,
            consensus_tx,
            max_blocks_fetch: max_block_fetch.unwrap_or(DEFAULT_MAX_BLOCKS_FETCH),
            poll_wait_ms: poll_wait_ms.unwrap_or(DEFAULT_POLL_WAIT_MS),
        };
        let task = ol_tracker_task(state, ctx);
        (handle, task)
    }

    pub(crate) fn ee_state_watcher(&self) -> watch::Receiver<EeAccountState> {
        self.ee_state_rx.clone()
    }

    pub fn consensus_watcher(&self) -> watch::Receiver<ConsensusHeads> {
        self.consensus_rx.clone()
    }
}
