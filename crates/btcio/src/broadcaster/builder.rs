use std::{
    fmt::{Debug, Formatter},
    sync::Arc,
    time::Duration,
};

use bitcoind_async_client::traits::{Broadcaster, Wallet};
use strata_service::{ServiceBuilder, ServiceMonitor, TickingInput, TokioMpscInput};
use strata_storage::BroadcastDbOps;
use strata_tasks::TaskExecutor;
use tokio::sync::mpsc;

use crate::{
    broadcaster::{
        input::BroadcasterInputMessage,
        io::BroadcasterIo,
        service::{BroadcasterService, BroadcasterStatus},
        state::BroadcasterServiceState,
    },
    BtcioParams,
};

/// Default broadcaster polling interval in milliseconds.
const DEFAULT_BROADCAST_POLL_INTERVAL_MS: u64 = 5_000;

/// Builder for launching the broadcaster service.
pub struct BroadcasterBuilder<T> {
    rpc_client: Arc<T>,
    ops: Arc<BroadcastDbOps>,
    config: BtcioParams,
    broadcast_poll_interval_ms: u64,
}

impl<T> Debug for BroadcasterBuilder<T> {
    #[expect(clippy::absolute_paths, reason = "qualified Result avoids ambiguity")]
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BroadcasterBuilder")
            .field("config", &self.config)
            .field(
                "broadcast_poll_interval_ms",
                &self.broadcast_poll_interval_ms,
            )
            .finish()
    }
}

impl<T> BroadcasterBuilder<T>
where
    T: Broadcaster + Wallet + Send + Sync + 'static,
{
    /// Creates a broadcaster builder with required dependencies.
    pub fn new(rpc_client: Arc<T>, ops: Arc<BroadcastDbOps>, config: BtcioParams) -> Self {
        Self {
            rpc_client,
            ops,
            config,
            broadcast_poll_interval_ms: DEFAULT_BROADCAST_POLL_INTERVAL_MS,
        }
    }

    pub fn with_broadcast_poll_interval_ms(mut self, broadcast_poll_interval_ms: u64) -> Self {
        self.broadcast_poll_interval_ms = broadcast_poll_interval_ms;
        self
    }

    /// Launches the broadcaster service and returns command sender, DB ops handle, and monitor.
    pub async fn launch(
        self,
        executor: &TaskExecutor,
    ) -> anyhow::Result<(
        mpsc::Sender<BroadcasterInputMessage>,
        Arc<BroadcastDbOps>,
        ServiceMonitor<BroadcasterStatus>,
    )> {
        let io = BroadcasterIo::new(self.rpc_client, self.ops.clone());
        let state = BroadcasterServiceState::try_new(io, self.config).await?;

        let (command_tx, command_rx) = mpsc::channel::<BroadcasterInputMessage>(64);
        let input = TickingInput::new(
            Duration::from_millis(self.broadcast_poll_interval_ms),
            TokioMpscInput::new(command_rx),
        );

        let monitor = ServiceBuilder::<BroadcasterService<BroadcasterIo<T>>, _>::new()
            .with_state(state)
            .with_input(input)
            .launch_async("l1_broadcaster", executor)
            .await?;

        Ok((command_tx, self.ops, monitor))
    }
}
