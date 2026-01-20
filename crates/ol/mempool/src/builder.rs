//! Mempool service builder for initialization and launch.

use std::{
    fmt::{Debug, Formatter},
    future::Future,
    sync::Arc,
};

use strata_identifiers::OLBlockCommitment;
use strata_ol_state_types::OLState;
use strata_service::{AsyncServiceInput, ServiceBuilder, ServiceInput};
use strata_status::{OLChainSyncUpdate, StatusChannel};
use strata_storage::NodeStorage;
use strata_tasks::TaskExecutor;
use tokio::sync::{mpsc, watch};

use crate::{
    MempoolCommand, MempoolHandle,
    service::MempoolService,
    state::{MempoolContext, MempoolServiceState},
    types::OLMempoolConfig,
};

/// Builder for creating and launching mempool service.
///
/// Separates service initialization logic from the handle interface.
pub struct MempoolBuilder {
    config: OLMempoolConfig,
    storage: Arc<NodeStorage>,
    status_channel: StatusChannel<OLState>,
    current_tip: OLBlockCommitment,
}

impl Debug for MempoolBuilder {
    #[expect(clippy::absolute_paths, reason = "qualified Result avoids ambiguity")]
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MempoolBuilder")
            .field("config", &self.config)
            .field("current_tip", &self.current_tip)
            .finish()
    }
}

impl MempoolBuilder {
    /// Create a new mempool builder.
    pub fn new(
        config: OLMempoolConfig,
        storage: Arc<NodeStorage>,
        status_channel: StatusChannel<OLState>,
        current_tip: OLBlockCommitment,
    ) -> Self {
        Self {
            config,
            storage,
            status_channel,
            current_tip,
        }
    }

    /// Launch the mempool service and return a handle.
    ///
    /// Creates the service with FCM chain sync integration via tokio::select!.
    pub async fn launch(self, texec: &TaskExecutor) -> anyhow::Result<MempoolHandle> {
        // Subscribe to chain sync updates
        let chain_sync_rx = self.status_channel.subscribe_chain_sync();

        let ctx = Arc::new(MempoolContext::new(
            self.config.clone(),
            self.storage.clone(),
        ));

        // Create mempool state with context and current tip
        let mut state =
            MempoolServiceState::new_with_context(ctx.clone(), self.current_tip).await?;

        // Load existing transactions from database
        state.load_from_db().await?;

        // Create command channel manually
        let (command_tx, command_rx) = mpsc::channel(self.config.command_buffer_size);

        // Create mempool input with fan-in
        let mempool_input = MempoolInput::new(command_rx, chain_sync_rx);

        // Launch service with mempool input
        let monitor = ServiceBuilder::<MempoolService<_>, _>::new()
            .with_state(state)
            .with_input(mempool_input)
            .launch_async("mempool", texec)
            .await?;

        Ok(MempoolHandle::new(command_tx, monitor))
    }
}

/// Input message type for the mempool service.
///
/// Supports both commands (from RPC/handle) and chain sync updates (from FCM).
#[derive(Debug)]
pub(crate) enum MempoolInputMessage {
    /// Command from RPC or handle
    Command(MempoolCommand),
    /// OL chain tip update from fork-choice manager
    ChainUpdate(OLChainSyncUpdate),
}

/// Mempool input that fans-in commands and OL chain sync updates.
struct MempoolInput {
    command_rx: mpsc::Receiver<MempoolCommand>,
    chain_sync_rx: watch::Receiver<Option<OLChainSyncUpdate>>,
    closed: bool,
}

impl MempoolInput {
    fn new(
        command_rx: mpsc::Receiver<MempoolCommand>,
        chain_sync_rx: watch::Receiver<Option<OLChainSyncUpdate>>,
    ) -> Self {
        Self {
            command_rx,
            chain_sync_rx,
            closed: false,
        }
    }
}

impl ServiceInput for MempoolInput {
    type Msg = MempoolInputMessage;
}

impl AsyncServiceInput for MempoolInput {
    // Cannot use `async fn` syntax: trait requires `-> impl Future` return type,
    // but `async fn` creates nested futures causing E0391 cyclic dependency error.
    // This pattern matches strata-service adapters (see TokioMpscInput, AsyncSyncInput).
    #[expect(
        clippy::manual_async_fn,
        reason = "async fn causes E0391 cyclic dependency"
    )]
    fn recv_next(&mut self) -> impl Future<Output = anyhow::Result<Option<Self::Msg>>> + Send {
        async move {
            loop {
                if self.closed {
                    return Ok(None);
                }

                tokio::select! {
                    biased;

                    // Chain sync update from FCM (checked first - maintains state consistency)
                    result = self.chain_sync_rx.changed() => {
                        match result {
                            Ok(()) => {
                                let update = self.chain_sync_rx.borrow_and_update().clone();
                                if let Some(chain_update) = update {
                                    return Ok(Some(MempoolInputMessage::ChainUpdate(chain_update)));
                                } else {
                                    // No update yet, loop back to wait for next message
                                    continue;
                                }
                            }
                            Err(_) => {
                                // Sender dropped, close the input
                                self.closed = true;
                                return Ok(None);
                            }
                        }
                    }
                    // Command from handle/RPC (checked second)
                    result = self.command_rx.recv() => {
                        match result {
                            Some(command) => return Ok(Some(MempoolInputMessage::Command(command))),
                            None => {
                                self.closed = true;
                                return Ok(None);
                            }
                        }
                    }
                }
            }
        }
    }
}
