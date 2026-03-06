use std::{
    sync::{Arc, atomic::AtomicU32},
    time::Duration,
};

use strata_btcio::writer::EnvelopeHandle;
use strata_consensus_logic::FcmServiceHandle;
use strata_identifiers::Buf32;
use strata_ol_block_assembly::BlockasmHandle;
use strata_service::{ServiceBuilder, ServiceMonitor};
use strata_status::StatusChannel;
use strata_storage::NodeStorage;
use strata_tasks::TaskExecutor;
use tokio::sync::mpsc;

use crate::sequencer::{
    input::SequencerTimerInput,
    service::{SequencerService, SequencerServiceState, SequencerServiceStatus},
};

/// Builder for the in-node sequencer service.
pub(crate) struct SequencerBuilder {
    blockasm_handle: Arc<BlockasmHandle>,
    envelope_handle: Arc<EnvelopeHandle>,
    storage: Arc<NodeStorage>,
    fcm_handle: Arc<FcmServiceHandle>,
    status_channel: Arc<StatusChannel>,
    sequencer_key: Buf32,
    poll_interval: Duration,
}

impl SequencerBuilder {
    #[expect(
        dead_code,
        reason = "used only after commit 2 switches startup to SequencerBuilder"
    )]
    pub(crate) fn new(
        blockasm_handle: Arc<BlockasmHandle>,
        envelope_handle: Arc<EnvelopeHandle>,
        storage: Arc<NodeStorage>,
        fcm_handle: Arc<FcmServiceHandle>,
        status_channel: Arc<StatusChannel>,
        sequencer_key: Buf32,
        poll_interval: Duration,
    ) -> Self {
        Self {
            blockasm_handle,
            envelope_handle,
            storage,
            fcm_handle,
            status_channel,
            sequencer_key,
            poll_interval,
        }
    }

    #[expect(
        dead_code,
        reason = "used only after commit 2 switches startup to SequencerBuilder"
    )]
    pub(crate) async fn launch(
        self,
        executor: &TaskExecutor,
    ) -> anyhow::Result<ServiceMonitor<SequencerServiceStatus>> {
        let active_duties = Arc::new(AtomicU32::new(0));
        let failed_duty_count = Arc::new(AtomicU32::new(0));
        let (failed_duties_tx, failed_duties_rx) = mpsc::channel(8);

        let state = SequencerServiceState {
            blockasm_handle: self.blockasm_handle,
            envelope_handle: self.envelope_handle,
            storage: self.storage,
            fcm_handle: self.fcm_handle,
            status_channel: self.status_channel,
            sequencer_key: self.sequencer_key,
            seen_duties: Default::default(),
            active_duties,
            failed_duty_count,
            failed_duties_tx,
            failed_duties_rx,
            duties_dispatched: 0,
        };

        let timer_input = SequencerTimerInput::new(self.poll_interval);

        ServiceBuilder::<SequencerService, _>::new()
            .with_state(state)
            .with_input(timer_input)
            .launch_async("ol_sequencer", executor)
            .await
    }
}
