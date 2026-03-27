//! Builder for launching the btcio bundler service.

use std::{sync::Arc, time::Duration};

use strata_db_types::types::IntentEntry;
use strata_service::{ServiceBuilder, ServiceMonitor};
use strata_storage::ops::writer::EnvelopeDataOps;
use strata_tasks::TaskExecutor;
use tokio::{sync::mpsc, time::interval};

use crate::writer::{
    bundler::get_initial_unbundled_entries,
    bundler_service::{BundlerInput, BundlerService, BundlerState, BundlerStatus},
};

pub(crate) struct BundlerBuilder {
    ops: Arc<EnvelopeDataOps>,
    bundle_interval: Duration,
    intent_rx: mpsc::Receiver<IntentEntry>,
}

impl BundlerBuilder {
    pub(crate) fn new(
        ops: Arc<EnvelopeDataOps>,
        bundle_interval: Duration,
        intent_rx: mpsc::Receiver<IntentEntry>,
    ) -> Self {
        Self {
            ops,
            bundle_interval,
            intent_rx,
        }
    }

    pub(crate) async fn launch(
        self,
        executor: &TaskExecutor,
    ) -> anyhow::Result<ServiceMonitor<BundlerStatus>> {
        let unbundled = get_initial_unbundled_entries(self.ops.as_ref())?;

        let state = BundlerState {
            ops: self.ops,
            unbundled,
        };
        let input = BundlerInput::new(interval(self.bundle_interval), self.intent_rx);

        ServiceBuilder::<BundlerService, _>::new()
            .with_state(state)
            .with_input(input)
            .launch_async("btcio_bundler", executor)
            .await
    }
}
