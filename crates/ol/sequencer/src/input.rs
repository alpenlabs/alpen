//! Timer-driven input for the sequencer service.

use std::time::Duration;

use strata_service::{AsyncServiceInput, ServiceInput};
use tokio::time::{self, Interval};

/// Timer-driven input for the sequencer service.
#[derive(Debug)]
pub struct SequencerTimerInput {
    interval: Interval,
}

impl SequencerTimerInput {
    pub fn new(poll_interval: Duration) -> Self {
        Self {
            interval: time::interval(poll_interval),
        }
    }
}

/// Events consumed by the sequencer service.
#[derive(Clone, Copy, Debug)]
pub enum SequencerEvent {
    Tick,
}

impl ServiceInput for SequencerTimerInput {
    type Msg = SequencerEvent;
}

impl AsyncServiceInput for SequencerTimerInput {
    async fn recv_next(&mut self) -> anyhow::Result<Option<Self::Msg>> {
        self.interval.tick().await;
        Ok(Some(SequencerEvent::Tick))
    }
}
