//! Timer-driven input for the sequencer service.

use std::time::Duration;

use strata_service::{AsyncServiceInput, ServiceInput};
use tokio::time::{self, Interval};

/// Timer-driven input for the sequencer service.
#[derive(Debug)]
pub struct SequencerTimerInput {
    duty_poll_interval: Interval,
    ol_block_interval: Interval,
}

impl SequencerTimerInput {
    pub fn new(duty_poll_interval: Duration, ol_block_interval: Duration) -> Self {
        Self {
            duty_poll_interval: time::interval(duty_poll_interval),
            ol_block_interval: time::interval(ol_block_interval),
        }
    }
}

/// Events consumed by the sequencer service.
#[derive(Clone, Copy, Debug)]
pub enum SequencerEvent {
    Tick,
    GenerationTick,
}

impl ServiceInput for SequencerTimerInput {
    type Msg = SequencerEvent;
}

impl AsyncServiceInput for SequencerTimerInput {
    async fn recv_next(&mut self) -> anyhow::Result<Option<Self::Msg>> {
        tokio::select! {
            _ = self.duty_poll_interval.tick() => Ok(Some(SequencerEvent::Tick)),
            _ = self.ol_block_interval.tick() => Ok(Some(SequencerEvent::GenerationTick)),
        }
    }
}
