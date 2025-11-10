use std::sync::Arc;

use alpen_ee_config::AlpenEeParams;
use strata_ee_acct_types::EeAccountState;
use tokio::sync::watch;

use crate::ol_tracker::ConsensusHeads;

pub(crate) struct OlTrackerCtx<TStorage, TOlClient> {
    pub(crate) storage: Arc<TStorage>,
    pub(crate) ol_client: Arc<TOlClient>,
    pub(crate) params: Arc<AlpenEeParams>,
    pub(crate) ee_state_tx: watch::Sender<EeAccountState>,
    pub(crate) consensus_tx: watch::Sender<ConsensusHeads>,
    pub(crate) max_blocks_fetch: u64,
    pub(crate) poll_wait_ms: u64,
    pub(crate) reorg_fetch_size: u64,
}

impl<TStorage, TOlClient> OlTrackerCtx<TStorage, TOlClient> {
    /// Notify watchers of latest state update.
    pub(crate) fn notify_state_update(&self, state: &EeAccountState) {
        let _ = self.ee_state_tx.send(state.clone());
    }

    /// Notify watchers of consensus state update.
    pub(crate) fn notify_consensus_update(&self, update: ConsensusHeads) {
        let _ = self.consensus_tx.send(update);
    }
}
