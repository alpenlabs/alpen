use std::sync::Arc;

use async_trait::async_trait;
use strata_state::sync_event::{EventSubmitter, SyncEvent};
use strata_storage::SyncEventManager;
use tokio::sync::mpsc;
use tracing::*;

use super::message::CsmMessage;

/// Controller handle for the consensus state machine.  Used to submit new sync
/// events for persistence and processing.
#[expect(missing_debug_implementations)]
pub struct CsmController {
    sync_ev_man: Arc<SyncEventManager>,
    csm_tx: mpsc::Sender<CsmMessage>,
}

impl CsmController {
    pub fn new(sync_ev_man: Arc<SyncEventManager>, csm_tx: mpsc::Sender<CsmMessage>) -> Self {
        Self {
            sync_ev_man,
            csm_tx,
        }
    }
}

#[async_trait]
impl EventSubmitter for CsmController {
    /// Writes a sync event to the database and updates the watch channel to
    /// trigger the CSM executor to process the event.
    fn submit_event(&self, sync_event: SyncEvent) -> anyhow::Result<()> {
        trace!(%sync_event, "writing sync event");
        let ev_idx = self
            .sync_ev_man
            .write_sync_event_blocking(sync_event.clone())?;
        let msg = CsmMessage::EventInput(ev_idx);
        if self.csm_tx.blocking_send(msg).is_err() {
            warn!(%ev_idx, "sync event receiver closed when submitting");
        } else {
            trace!(%ev_idx, %sync_event, "sent csm event input");
        }

        Ok(())
    }

    /// Writes a sync event to the database and updates the watch channel to
    /// trigger the CSM executor to process the event.
    async fn submit_event_async(&self, sync_event: SyncEvent) -> anyhow::Result<()> {
        trace!(%sync_event, "writing sync event");
        let ev_idx = self
            .sync_ev_man
            .write_sync_event_async(sync_event.clone())
            .await?;
        let msg = CsmMessage::EventInput(ev_idx);
        if self.csm_tx.send(msg).await.is_err() {
            warn!(%ev_idx, "sync event receiver closed when submitting");
        } else {
            trace!(%ev_idx, %sync_event, "sent csm event input");
        }

        Ok(())
    }
}
