use std::sync::Arc;

use alpen_ee_common::ConsensusHeads;
use alpen_ee_config::AlpenEeParams;
use strata_ee_acct_types::EeAccountState;
use tokio::sync::watch;

pub(crate) struct OLTrackerCtx<TStorage, TOLClient> {
    pub storage: Arc<TStorage>,
    pub ol_client: Arc<TOLClient>,
    pub params: Arc<AlpenEeParams>,
    pub ee_state_tx: watch::Sender<EeAccountState>,
    pub consensus_tx: watch::Sender<ConsensusHeads>,
    pub max_blocks_fetch: u64,
    pub poll_wait_ms: u64,
    pub reorg_fetch_size: u64,
}

impl<TStorage, TOLClient> OLTrackerCtx<TStorage, TOLClient> {
    /// Notify watchers of latest state update.
    pub(crate) fn notify_state_update(&self, state: &EeAccountState) {
        let _ = self.ee_state_tx.send(state.clone());
    }

    /// Notify watchers of consensus state update.
    pub(crate) fn notify_consensus_update(&self, update: ConsensusHeads) {
        let _ = self.consensus_tx.send(update);
    }
}
