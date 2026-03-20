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
}

impl<TStorage, TOLClient> OLTrackerCtx<TStorage, TOLClient> {
    /// Notify watchers of latest state update.
    pub(crate) fn notify_ol_status_update(&self, status: OLFinalizedStatus) {
        let _ = self.ol_status_tx.send_if_modified(|current| {
            if *current == status {
                false
            } else {
                *current = status;
                true
            }
        });
    }

    /// Notify watchers of consensus state update.
    pub(crate) fn notify_consensus_update(&self, update: ConsensusHeads) {
        let _ = self.consensus_tx.send_if_modified(|current| {
            if *current == update {
                false
            } else {
                *current = update.clone();
                true
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use alpen_ee_common::{ConsensusHeads, OLFinalizedStatus};
    use strata_identifiers::{Buf32, OLBlockCommitment, OLBlockId};
    use tokio::sync::watch;

    use super::*;

    fn make_status(slot: u64, ee_byte: u8) -> OLFinalizedStatus {
        OLFinalizedStatus {
            ol_block: OLBlockCommitment::new(slot, OLBlockId::from(Buf32::new([slot as u8; 32]))),
            last_ee_block: [ee_byte; 32].into(),
        }
    }

    #[test]
    fn skips_duplicate_ol_status_notifications() {
        let initial = make_status(1, 1);
        let (ol_status_tx, mut ol_status_rx) = watch::channel(initial);
        let (consensus_tx, _consensus_rx) = watch::channel(ConsensusHeads {
            confirmed: [1u8; 32].into(),
            finalized: [2u8; 32].into(),
        });
        let ctx = OLTrackerCtx::<(), ()> {
            storage: Arc::new(()),
            ol_client: Arc::new(()),
            genesis_epoch: 0,
            ol_status_tx,
            consensus_tx,
            max_epochs_fetch: 1,
            poll_wait_ms: 1_000,
        };

        ctx.notify_ol_status_update(initial);
        assert!(!ol_status_rx.has_changed().unwrap());

        let next = make_status(2, 2);
        ctx.notify_ol_status_update(next);
        assert!(ol_status_rx.has_changed().unwrap());
        assert_eq!(*ol_status_rx.borrow_and_update(), next);
    }
}
