//! Service input for the checkpoint sync service.

use strata_csm_types::CheckpointState;
use strata_service::{AsyncServiceInput, ServiceInput};
use tokio::sync::watch;
use tracing::{debug, warn};

/// Input source that wakes the service on each new CSM client state update.
#[derive(Debug)]
pub(crate) struct CheckpointSyncInput {
    /// Receiver for CSM-published client state updates.
    clstate_rx: watch::Receiver<CheckpointState>,
}

impl CheckpointSyncInput {
    pub(crate) fn new(clstate_rx: watch::Receiver<CheckpointState>) -> Self {
        Self { clstate_rx }
    }
}

/// Input event for the checkpoint sync service.
#[derive(Clone, Debug)]
pub enum CheckpointSyncEvent {
    /// A new CSM client state was observed.
    NewCsmStateUpdate,
    /// The client state channel closed; the service should shut down.
    Abort,
}

impl ServiceInput for CheckpointSyncInput {
    type Msg = CheckpointSyncEvent;
}

impl AsyncServiceInput for CheckpointSyncInput {
    async fn recv_next(&mut self) -> anyhow::Result<Option<Self::Msg>> {
        let msg = match self.clstate_rx.changed().await {
            Ok(()) => {
                debug!("received new client state update");
                CheckpointSyncEvent::NewCsmStateUpdate
            }
            Err(e) => {
                warn!(%e, "ClientState update channel closed");
                CheckpointSyncEvent::Abort
            }
        };
        Ok(Some(msg))
    }
}
