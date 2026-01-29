//! Service input for the OL checkpoint builder.

use std::future::Future;

use strata_primitives::epoch::EpochCommitment;
use strata_service::{AsyncServiceInput, ServiceInput};
use tokio::sync::broadcast;
use tracing::trace;

use crate::message::OLCheckpointMessage;

/// Input source for the checkpoint service.
#[derive(Debug)]
pub struct OLCheckpointInput {
    summary_rx: broadcast::Receiver<EpochCommitment>,
    closed: bool,
}

impl OLCheckpointInput {
    pub fn new(summary_rx: broadcast::Receiver<EpochCommitment>) -> Self {
        Self {
            summary_rx,
            closed: false,
        }
    }
}

impl ServiceInput for OLCheckpointInput {
    type Msg = OLCheckpointMessage;
}

impl AsyncServiceInput for OLCheckpointInput {
    #[expect(
        clippy::manual_async_fn,
        reason = "async fn causes E0391 cyclic dependency in this trait"
    )]
    fn recv_next(&mut self) -> impl Future<Output = anyhow::Result<Option<Self::Msg>>> + Send {
        async move {
            if self.closed {
                return Ok(Some(OLCheckpointMessage::Abort));
            }

            loop {
                match self.summary_rx.recv().await {
                    Ok(commitment) => {
                        return Ok(Some(OLCheckpointMessage::NewEpochSummary(commitment)));
                    }
                    Err(broadcast::error::RecvError::Lagged(count)) => {
                        trace!(skipped = count, "checkpoint input lagged; retrying");
                        continue;
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        self.closed = true;
                        trace!("epoch summary channel closed");
                        return Ok(Some(OLCheckpointMessage::Abort));
                    }
                }
            }
        }
    }
}
