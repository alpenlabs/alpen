//! Timer-driven input for the sequencer service.

use std::time::Duration;

use strata_service::{AsyncServiceInput, ServiceInput};
use tokio::time::{self, Interval, MissedTickBehavior};

/// Timer-driven input for the sequencer service.
#[derive(Debug)]
pub struct SequencerTimerInput {
    ol_block_interval: Interval,
}

impl SequencerTimerInput {
    pub fn new(ol_block_interval: Duration) -> Self {
        let mut ol_block_interval = time::interval(ol_block_interval);
        ol_block_interval.set_missed_tick_behavior(MissedTickBehavior::Delay);

        Self { ol_block_interval }
    }
}

/// Events consumed by the sequencer service.
#[derive(Clone, Copy, Debug)]
pub enum SequencerEvent {
    GenerationTick,
}

impl ServiceInput for SequencerTimerInput {
    type Msg = SequencerEvent;
}

impl AsyncServiceInput for SequencerTimerInput {
    async fn recv_next(&mut self) -> anyhow::Result<Option<Self::Msg>> {
        self.ol_block_interval.tick().await;
        Ok(Some(SequencerEvent::GenerationTick))
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::time::MissedTickBehavior;

    use super::*;

    #[tokio::test]
    async fn timer_uses_delay_for_missed_ticks() {
        let input = SequencerTimerInput::new(Duration::from_secs(1));

        assert_eq!(
            input.ol_block_interval.missed_tick_behavior(),
            MissedTickBehavior::Delay
        );
    }
}
