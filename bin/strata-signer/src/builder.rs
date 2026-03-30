//! Builder for launching the signer service.

use std::{sync::Arc, time::Duration};

use strata_common::ws_client::ManagedWsClient;
use strata_service::{ServiceBuilder, ServiceMonitor};
use strata_tasks::TaskExecutor;
use tokio::sync::mpsc;

use crate::{
    helpers::SequencerSk,
    input::SignerInput,
    service::{SignerService, SignerServiceState, SignerServiceStatus},
};

/// Capacity of the internal channel used to report failed duties back to the
/// service for retry.
const FAILED_DUTY_CHANNEL_CAPACITY: usize = 64;

/// Builder for the signer service.
pub(crate) struct SignerBuilder {
    rpc: Arc<ManagedWsClient>,
    sequencer_key: SequencerSk,
    duty_poll_interval: Duration,
}

impl SignerBuilder {
    pub(crate) fn new(
        rpc: Arc<ManagedWsClient>,
        sequencer_key: SequencerSk,
        duty_poll_interval: Duration,
    ) -> Self {
        Self {
            rpc,
            sequencer_key,
            duty_poll_interval,
        }
    }

    pub(crate) async fn launch(
        self,
        executor: &TaskExecutor,
    ) -> anyhow::Result<ServiceMonitor<SignerServiceStatus>> {
        let (failed_tx, failed_rx) = mpsc::channel(FAILED_DUTY_CHANNEL_CAPACITY);

        let state = SignerServiceState::new(self.rpc, self.sequencer_key, failed_tx);
        let input = SignerInput::new(self.duty_poll_interval, failed_rx);

        ServiceBuilder::<SignerService, _>::new()
            .with_state(state)
            .with_input(input)
            .launch_async("strata_signer", executor)
            .await
    }
}
