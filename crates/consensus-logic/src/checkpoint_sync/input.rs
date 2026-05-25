//! Service input for the checkpoint sync service.

use strata_csm_types::{CheckpointState, ClientState};
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
        let msg = wait_for_client_change(&mut self.clstate_rx)
            .await
            .map(|_| CheckpointSyncEvent::NewCsmStateUpdate)
            .inspect(|v| debug!(client_state = ?v, "received new client state update"))
            .unwrap_or_else(|e| {
                warn!("ClientState update channel closed: {e}");
                CheckpointSyncEvent::Abort
            });
        Ok(Some(msg))
    }
}

/// Waits until there's a new client state and returns it.
async fn wait_for_client_change(
    cl_rx: &mut watch::Receiver<CheckpointState>,
) -> Result<ClientState, watch::error::RecvError> {
    cl_rx.changed().await?;
    let state = cl_rx.borrow_and_update().clone();
    Ok(state.client_state)
}
