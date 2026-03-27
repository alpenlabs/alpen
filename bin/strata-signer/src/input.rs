//! Input source for the signer service.
//!
//! Merges two event streams via `select!`: a periodic poll timer and a
//! channel that receives duty IDs whose signing tasks failed.

use std::time::Duration;

use strata_primitives::buf::Buf32;
use strata_service::{AsyncServiceInput, ServiceInput};
use tokio::{
    sync::mpsc,
    time::{Interval, interval},
};

/// Events processed by the signer service.
#[derive(Debug)]
pub(crate) enum SignerEvent {
    /// Periodic tick to poll for new duties.
    PollTick,
    /// A duty whose signing task failed, eligible for retry.
    DutyFailed(Buf32),
}

/// Fan-in input that merges a periodic timer with a failure notification channel.
pub(crate) struct SignerInput {
    interval: Interval,
    failed_rx: mpsc::Receiver<Buf32>,
}

impl SignerInput {
    pub(crate) fn new(poll_interval: Duration, failed_rx: mpsc::Receiver<Buf32>) -> Self {
        Self {
            interval: interval(poll_interval),
            failed_rx,
        }
    }
}

impl ServiceInput for SignerInput {
    type Msg = SignerEvent;
}

impl AsyncServiceInput for SignerInput {
    async fn recv_next(&mut self) -> anyhow::Result<Option<Self::Msg>> {
        tokio::select! {
            _ = self.interval.tick() => Ok(Some(SignerEvent::PollTick)),
            Some(duty_id) = self.failed_rx.recv() => Ok(Some(SignerEvent::DutyFailed(duty_id))),
        }
    }
}
