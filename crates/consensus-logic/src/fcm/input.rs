use strata_csm_types::{CheckpointState, ClientState};
use strata_service::{ServiceInput, SyncServiceInput};
use tokio::{
    runtime::Handle,
    sync::{mpsc, watch},
};
use tracing::trace;

use crate::message::ForkChoiceMessage;

#[expect(clippy::large_enum_variant, reason = "used for fork choice manager")]
#[derive(Clone, Debug)]
pub enum FcmEvent {
    NewFcmMsg(ForkChoiceMessage),
    NewStateUpdate(ClientState),
    Abort,
}

#[derive(Debug)]
pub struct FcmInput {
    handle: Handle,
    fcm_rx: mpsc::Receiver<ForkChoiceMessage>,
    // TODO: Rename CheckpointState to sth like ClientStateAtL1
    clstate_rx: watch::Receiver<CheckpointState>,
}

impl FcmInput {
    pub fn new(
        handle: Handle,
        fcm_rx: mpsc::Receiver<ForkChoiceMessage>,
        clstate_rx: watch::Receiver<CheckpointState>,
    ) -> Self {
        Self {
            handle,
            fcm_rx,
            clstate_rx,
        }
    }
}

impl ServiceInput for FcmInput {
    type Msg = FcmEvent;
}

impl SyncServiceInput for FcmInput {
    fn recv_next(&mut self) -> anyhow::Result<Option<Self::Msg>> {
        let msg = self.handle.block_on(async {
            tokio::select! {
                m = self.fcm_rx.recv() => {
                    let msg = m.map(FcmEvent::NewFcmMsg).unwrap_or_else(|| {
                        trace!("input channel closed");
                        FcmEvent::Abort
                    });
                    Some(msg)
                }
                c = wait_for_client_change(&mut self.clstate_rx) => {
                    let msg = c.map(FcmEvent::NewStateUpdate).unwrap_or_else(|_| {
                        trace!("ClientState update channel closed");
                        FcmEvent::Abort
                    });
                    Some(msg)
                }
            }
        });
        Ok(msg)
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
