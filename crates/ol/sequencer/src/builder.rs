//! Builder for launching the sequencer service.

use std::{
    sync::{atomic::AtomicU32, Arc},
    time::Duration,
};

use strata_primitives::buf::Buf32;
use strata_service::{ServiceBuilder, ServiceMonitor};
use strata_tasks::TaskExecutor;
use tokio::sync::mpsc;

use crate::{
    input::SequencerTimerInput,
    service::{SequencerContext, SequencerService, SequencerServiceState, SequencerServiceStatus},
};

/// Builder for the sequencer service, generic over the context implementation.
pub struct SequencerBuilder<C: SequencerContext> {
    context: Arc<C>,
    sequencer_key: Buf32,
    poll_interval: Duration,
}

impl<C: SequencerContext> SequencerBuilder<C> {
    pub fn new(context: Arc<C>, sequencer_key: Buf32, poll_interval: Duration) -> Self {
        Self {
            context,
            sequencer_key,
            poll_interval,
        }
    }

    pub async fn launch(
        self,
        executor: &TaskExecutor,
    ) -> anyhow::Result<ServiceMonitor<SequencerServiceStatus>> {
        let active_duties = Arc::new(AtomicU32::new(0));
        let failed_duty_count = Arc::new(AtomicU32::new(0));
        let (failed_duties_tx, failed_duties_rx) = mpsc::channel(8);

        let state = SequencerServiceState::new(
            self.context,
            self.sequencer_key,
            active_duties,
            failed_duty_count,
            failed_duties_tx,
            failed_duties_rx,
        );

        let timer_input = SequencerTimerInput::new(self.poll_interval);

        ServiceBuilder::<SequencerService<C>, _>::new()
            .with_state(state)
            .with_input(timer_input)
            .launch_async("ol_sequencer", executor)
            .await
    }
}
