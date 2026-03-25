//! Builder for launching the sequencer service.

use std::{sync::Arc, time::Duration};

use strata_service::{ServiceBuilder, ServiceMonitor};
use strata_tasks::TaskExecutor;

use crate::{
    input::SequencerTimerInput,
    service::{SequencerContext, SequencerService, SequencerServiceState, SequencerServiceStatus},
};

/// Builder for the sequencer service, generic over the context implementation.
pub struct SequencerBuilder<C: SequencerContext> {
    context: Arc<C>,
    ol_block_interval: Duration,
}

impl<C: SequencerContext> SequencerBuilder<C> {
    pub fn new(context: Arc<C>, ol_block_interval: Duration) -> Self {
        Self {
            context,
            ol_block_interval,
        }
    }

    pub async fn launch(
        self,
        executor: &TaskExecutor,
    ) -> anyhow::Result<ServiceMonitor<SequencerServiceStatus>> {
        let state = SequencerServiceState::new(self.context);
        let timer_input = SequencerTimerInput::new(self.ol_block_interval);

        ServiceBuilder::<SequencerService<C>, _>::new()
            .with_state(state)
            .with_input(timer_input)
            .launch_async("ol_sequencer", executor)
            .await
    }
}
