//! Builder for launching the btcio watcher service.

use std::{sync::Arc, time::Duration};

use bitcoind_async_client::traits::{Reader, Signer, Wallet};
use strata_service::{ServiceBuilder, ServiceMonitor, TickingInput, TokioMpscInput};
use strata_storage::ops::writer::EnvelopeDataOps;
use strata_tasks::TaskExecutor;
use tokio::sync::mpsc;

use crate::{
    broadcaster::L1BroadcastHandle,
    writer::{
        context::WriterContext,
        handle::get_next_payloadidx_to_watch,
        watcher_service::{WatcherContextImpl, WatcherService, WatcherState, WatcherStatus},
    },
};

#[expect(missing_debug_implementations, reason = "inner types lack Debug")]
pub struct WatcherBuilder<R: Reader + Signer + Wallet + Send + Sync + 'static> {
    context: Arc<WriterContext<R>>,
    ops: Arc<EnvelopeDataOps>,
    broadcast_handle: Arc<L1BroadcastHandle>,
    poll_dur: Duration,
}

impl<R: Reader + Signer + Wallet + Send + Sync + 'static> WatcherBuilder<R> {
    pub fn new(
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

    pub async fn launch(
        self,
        executor: &TaskExecutor,
    ) -> anyhow::Result<ServiceMonitor<WatcherStatus>> {
        let curr_payloadidx = get_next_payloadidx_to_watch(self.ops.as_ref())?;

        let ctx = WatcherContextImpl::new(self.context, self.ops, self.broadcast_handle);

        // The tick channel is never written to; the sender lives in state to keep
        // the channel open so TickingInput never sees a closed inner.
        let (tick_guard, tick_rx) = mpsc::channel::<()>(1);

        let state = WatcherState::new(ctx, curr_payloadidx, tick_guard);
        let input = TickingInput::new(self.poll_dur, TokioMpscInput::new(tick_rx));

        ServiceBuilder::<WatcherService<WatcherContextImpl<R>>, _>::new()
            .with_state(state)
            .with_input(input)
            .launch_async("btcio_watcher", executor)
            .await
    }
}
