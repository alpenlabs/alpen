//! Builder for launching the btcio watcher service.

use std::{sync::Arc, time::Duration};

use bitcoind_async_client::traits::{Reader, Signer, Wallet};
use strata_service::{DumbTickHandle, DumbTickingInput, ServiceBuilder, ServiceMonitor};
use strata_storage::ops::writer::EnvelopeDataOps;
use strata_tasks::TaskExecutor;

use super::service::{WatcherContextImpl, WatcherService, WatcherState, WatcherStatus};
use crate::{
    broadcaster::L1BroadcastHandle,
    writer::{context::WriterContext, handle::get_next_payloadidx_to_watch},
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
    ) -> anyhow::Result<(DumbTickHandle, ServiceMonitor<WatcherStatus>)> {
        let curr_payloadidx = get_next_payloadidx_to_watch(self.ops.as_ref())?;

        let ctx = WatcherContextImpl::new(self.context, self.ops, self.broadcast_handle);
        let (stop_handle, input) = DumbTickingInput::new(self.poll_dur);
        let state = WatcherState::new(ctx, curr_payloadidx);

        let monitor = ServiceBuilder::<WatcherService<WatcherContextImpl<R>>, _>::new()
            .with_state(state)
            .with_input(input)
            .launch_async("btcio_watcher", executor)
            .await?;

        Ok((stop_handle, monitor))
    }
}
