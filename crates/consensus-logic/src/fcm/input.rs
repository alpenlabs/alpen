use strata_csm_types::CheckpointState;
use strata_service::{AsyncServiceInput, ServiceInput};
use tokio::sync::{mpsc, watch};
use tracing::trace;

use crate::message::ForkChoiceMessage;

#[derive(Clone, Debug)]
pub enum FcmEvent {
    NewFcmMsg(ForkChoiceMessage),
    NewStateUpdate,
    Abort,
}

#[derive(Debug)]
pub struct FcmInput {
    fcm_rx: mpsc::Receiver<ForkChoiceMessage>,
    // TODO: Rename CheckpointState to sth like ClientStateAtL1
    clstate_rx: watch::Receiver<CheckpointState>,
}

impl FcmInput {
    pub fn new(
        fcm_rx: mpsc::Receiver<ForkChoiceMessage>,
        clstate_rx: watch::Receiver<CheckpointState>,
    ) -> Self {
        Self { fcm_rx, clstate_rx }
    }
}

impl ServiceInput for FcmInput {
    type Msg = FcmEvent;
}

impl AsyncServiceInput for FcmInput {
    async fn recv_next(&mut self) -> anyhow::Result<Option<Self::Msg>> {
        let msg = tokio::select! {
            m = self.fcm_rx.recv() => {
                let msg = m.map(FcmEvent::NewFcmMsg).unwrap_or_else(|| {
                    trace!("input channel closed");
                    FcmEvent::Abort
                });
                Some(msg)
            }
            c = wait_for_client_change(&mut self.clstate_rx) => {
                let msg = c.map(|_| FcmEvent::NewStateUpdate).unwrap_or_else(|_| {
                    trace!("ClientState update channel closed");
                    FcmEvent::Abort
                });
                Some(msg)
            }
        };
        Ok(msg)
    }
}

/// Waits until there's a new client state update.
async fn wait_for_client_change(
    cl_rx: &mut watch::Receiver<CheckpointState>,
) -> Result<(), watch::error::RecvError> {
    cl_rx.changed().await?;
    cl_rx.borrow_and_update();
    Ok(())
}
