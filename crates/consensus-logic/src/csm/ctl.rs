use std::sync::Arc;

use async_trait::async_trait;
use strata_primitives::l1::L1BlockCommitment;
use strata_state::sync_event::BlockSubmitter;
use tokio::sync::mpsc;
use tracing::*;

/// Controller handle for the consensus state machine.  Used to submit new sync
/// events for persistence and processing.
#[expect(missing_debug_implementations)]
pub struct CsmController {
    csm_tx: mpsc::Sender<L1BlockCommitment>,
}

impl CsmController {
    pub fn new(csm_tx: mpsc::Sender<L1BlockCommitment>) -> Self {
        Self { csm_tx }
    }
}

#[async_trait]
impl BlockSubmitter for CsmController {
    /// Writes a sync event to the database and updates the watch channel to
    /// trigger the CSM executor to process the event.
    fn submit_event(&self, sync_event: L1BlockCommitment) -> anyhow::Result<()> {
        trace!(%sync_event, "submitting sync event");
        if self.csm_tx.blocking_send(sync_event).is_err() {
            warn!(%sync_event, "sync event receiver closed when submitting");
        } else {
            trace!(%sync_event, "sent csm event input");
        }

        Ok(())
    }

    /// Writes a sync event to the database and updates the watch channel to
    /// trigger the CSM executor to process the event.
    async fn submit_event_async(&self, sync_event: L1BlockCommitment) -> anyhow::Result<()> {
        trace!(%sync_event, "submitting sync event");
        if self.csm_tx.send(sync_event).await.is_err() {
            warn!(%sync_event, "sync event receiver closed when submitting");
        } else {
            trace!(%sync_event, "sent csm event input");
        }

        Ok(())
    }
}
