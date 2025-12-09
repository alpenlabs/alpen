use std::sync::Arc;

use alpen_ee_common::{ConsensusHeads, OLFinalizedStatus};
use alpen_ee_config::AlpenEeParams;
use tokio::sync::watch;

pub(crate) struct OLTrackerCtx<TStorage, TOLClient> {
    pub storage: Arc<TStorage>,
    pub ol_client: Arc<TOLClient>,
    pub params: Arc<AlpenEeParams>,
    pub ol_status_tx: watch::Sender<OLFinalizedStatus>,
    pub consensus_tx: watch::Sender<ConsensusHeads>,
    pub max_epochs_fetch: u32,
    pub poll_wait_ms: u64,
}

impl<TStorage, TOLClient> OLTrackerCtx<TStorage, TOLClient> {
    /// Notify watchers of latest state update.
    pub(crate) fn notify_ol_status_update(&self, status: OLFinalizedStatus) {
        let _ = self.ol_status_tx.send(status);
    }

    /// Notify watchers of consensus state update.
    pub(crate) fn notify_consensus_update(&self, update: ConsensusHeads) {
        let _ = self.consensus_tx.send(update);
    }
}
