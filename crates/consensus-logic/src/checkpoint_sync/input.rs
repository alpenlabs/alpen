use strata_csm_types::{CheckpointState, ClientState};
use strata_service::{AsyncServiceInput, ServiceInput};
use tokio::sync::watch;
use tracing::trace;

#[derive(Debug)]
pub(super) struct CheckpointSyncInput {
    clstate_rx: watch::Receiver<CheckpointState>,
}

impl CheckpointSyncInput {
    pub(super) fn new(clstate_rx: watch::Receiver<CheckpointState>) -> Self {
        Self { clstate_rx }
    }
}

#[expect(clippy::large_enum_variant, reason = "Avoiding Boxes")]
#[derive(Clone, Debug)]
pub enum CheckpointSyncEvent {
    NewStateUpdate(ClientState),
    Abort,
}

impl ServiceInput for CheckpointSyncInput {
    type Msg = CheckpointSyncEvent;
}

impl AsyncServiceInput for CheckpointSyncInput {
    async fn recv_next(&mut self) -> anyhow::Result<Option<Self::Msg>> {
        let msg = wait_for_client_change(&mut self.clstate_rx)
            .await
            .map(CheckpointSyncEvent::NewStateUpdate)
            .unwrap_or_else(|e| {
                trace!("ClientState update channel closed: {e}");
                CheckpointSyncEvent::Abort
            });
        Ok(Some(msg))
    }
}

/// Waits until there's a new client state and returns the client state.
async fn wait_for_client_change(
    cl_rx: &mut watch::Receiver<CheckpointState>,
) -> Result<ClientState, watch::error::RecvError> {
    cl_rx.changed().await?;
    let state = cl_rx.borrow_and_update().clone();
    Ok(state.client_state)
}
