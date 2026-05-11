use std::sync::Arc;

use alpen_ee_common::{ConsensusHeads, OLFinalizedStatus};
use tokio::sync::watch;

pub(crate) struct OLTrackerCtx<TStorage, TOLClient> {
    pub storage: Arc<TStorage>,
    pub ol_client: Arc<TOLClient>,
    pub genesis_epoch: u32,
    pub ol_status_tx: watch::Sender<OLFinalizedStatus>,
    pub consensus_tx: watch::Sender<ConsensusHeads>,
    pub max_epochs_fetch: u32,
    pub poll_wait_ms: u64,
    /// Dev/test only. When true, the tracker advances on the OL's
    /// `finalized` epoch (FCM-based, no L1 round-trip required) rather
    /// than the canonical `confirmed` epoch (CSM-based, depends on
    /// checkpoint observation on L1). Lets test environments where
    /// the CSM checkpoint pipeline can't keep up with rapid SAU
    /// emission (e.g. `--dev-native-noop-prover`) still consume
    /// inbox messages without waiting for L1 confirmations.
    pub track_finalized_epoch: bool,
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
