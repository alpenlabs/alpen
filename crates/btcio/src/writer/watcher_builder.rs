//! Builder for launching the btcio watcher service.

use std::{collections::HashMap, sync::Arc, time::Duration};

use bitcoind_async_client::traits::{Reader, Signer, Wallet};
use strata_service::{ServiceBuilder, ServiceMonitor};
use strata_storage::ops::writer::EnvelopeDataOps;
use strata_tasks::TaskExecutor;

use crate::{
    broadcaster::L1BroadcastHandle,
    writer::{
        context::WriterContext,
        handle::get_next_payloadidx_to_watch,
        watcher_service::{WatcherInput, WatcherService, WatcherState, WatcherStatus},
    },
};

pub(crate) struct WatcherBuilder<R: Reader + Signer + Wallet + Send + Sync + 'static> {
    context: Arc<WriterContext<R>>,
    ops: Arc<EnvelopeDataOps>,
    broadcast_handle: Arc<L1BroadcastHandle>,
    poll_dur: Duration,
}

impl<R: Reader + Signer + Wallet + Send + Sync + 'static> WatcherBuilder<R> {
    pub(crate) fn new(
        context: Arc<WriterContext<R>>,
        ops: Arc<EnvelopeDataOps>,
        broadcast_handle: Arc<L1BroadcastHandle>,
        poll_dur: Duration,
    ) -> Self {
        Self {
            context,
            ops,
            broadcast_handle,
            poll_dur,
        }
    }

    pub(crate) async fn launch(
        self,
        executor: &TaskExecutor,
    ) -> anyhow::Result<ServiceMonitor<WatcherStatus>> {
        let curr_payloadidx = get_next_payloadidx_to_watch(self.ops.as_ref())?;

        let state = WatcherState {
            context: self.context,
            ops: self.ops,
            broadcast_handle: self.broadcast_handle,
            unsigned_cache: HashMap::new(),
            curr_payloadidx,
        };
        let input = WatcherInput::new(self.poll_dur);

        ServiceBuilder::<WatcherService<R>, _>::new()
            .with_state(state)
            .with_input(input)
            .launch_async("btcio_watcher", executor)
            .await
    }
}
