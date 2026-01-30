//! Builder pattern for launching the OL checkpoint service.

use std::sync::Arc;

use strata_node_context::NodeContext;
use strata_primitives::epoch::EpochCommitment;
use strata_service::{ServiceBuilder, SyncAsyncInput};
use strata_status::StatusChannel;
use strata_storage::NodeStorage;
use strata_tasks::TaskExecutor;
use tokio::{runtime::Handle, sync::broadcast};

use crate::{
    errors::{OLCheckpointError, WorkerResult},
    handle::OLCheckpointHandle,
    input::OLCheckpointInput,
    providers::{
        DaProvider, EmptyDaProvider, EmptyLogProvider, LogProvider, PlaceholderProofProvider,
        ProofProvider,
    },
    service::OLCheckpointService,
    state::OLCheckpointServiceState,
};

#[expect(
    missing_debug_implementations,
    reason = "Some inner types don't have Debug implementation"
)]
/// Builder for constructing and launching an OL checkpoint service.
pub struct OLCheckpointBuilder {
    storage: Option<Arc<NodeStorage>>,
    status_channel: Option<StatusChannel>,
    runtime_handle: Option<Handle>,
    epoch_summary_rx: Option<broadcast::Receiver<EpochCommitment>>,
    da_provider: Option<Arc<dyn DaProvider>>,
    log_provider: Option<Arc<dyn LogProvider>>,
    proof_provider: Option<Arc<dyn ProofProvider>>,
}

impl OLCheckpointBuilder {
    /// Create a new builder instance.
    pub fn new() -> Self {
        Self {
            storage: None,
            status_channel: None,
            runtime_handle: None,
            epoch_summary_rx: None,
            da_provider: None,
            log_provider: None,
            proof_provider: None,
        }
    }

    /// Set storage, status channel, and runtime handle from [`NodeContext`].
    pub fn with_node_context(mut self, nodectx: &NodeContext) -> Self {
        self.storage = Some(nodectx.storage().clone());
        self.status_channel = Some(nodectx.status_channel().as_ref().clone());
        self.runtime_handle = Some(nodectx.executor().handle().clone());
        self
    }

    /// Set the epoch summary receiver for driving checkpoint creation.
    pub fn with_epoch_summary_receiver(
        mut self,
        receiver: broadcast::Receiver<EpochCommitment>,
    ) -> Self {
        self.epoch_summary_rx = Some(receiver);
        self
    }

    /// Set a custom DA provider. Defaults to [`EmptyDaProvider`].
    pub fn with_da_provider(mut self, provider: Arc<dyn DaProvider>) -> Self {
        self.da_provider = Some(provider);
        self
    }

    /// Set a custom log provider. Defaults to [`EmptyLogProvider`].
    pub fn with_log_provider(mut self, provider: Arc<dyn LogProvider>) -> Self {
        self.log_provider = Some(provider);
        self
    }

    /// Set a custom proof provider. Defaults to [`PlaceholderProofProvider`].
    pub fn with_proof_provider(mut self, provider: Arc<dyn ProofProvider>) -> Self {
        self.proof_provider = Some(provider);
        self
    }

    /// Launch the OL checkpoint service and return a handle to it.
    pub fn launch(self, executor: &TaskExecutor) -> WorkerResult<OLCheckpointHandle> {
        let storage = self
            .storage
            .ok_or(OLCheckpointError::MissingDependency("storage"))?;
        let status_channel = self
            .status_channel
            .ok_or(OLCheckpointError::MissingDependency("status_channel"))?;
        let runtime_handle = self
            .runtime_handle
            .ok_or(OLCheckpointError::MissingDependency("runtime_handle"))?;
        let epoch_summary_rx = self
            .epoch_summary_rx
            .ok_or(OLCheckpointError::MissingDependency("epoch_summary_rx"))?;

        // Use v1 defaults if no custom providers specified
        let da_provider = self
            .da_provider
            .unwrap_or_else(|| Arc::new(EmptyDaProvider));
        let log_provider = self
            .log_provider
            .unwrap_or_else(|| Arc::new(EmptyLogProvider));
        let proof_provider = self
            .proof_provider
            .unwrap_or_else(|| Arc::new(PlaceholderProofProvider));

        let input = OLCheckpointInput::new(epoch_summary_rx);
        let input = SyncAsyncInput::new(input, runtime_handle.clone());

        let state = OLCheckpointServiceState::new(
            storage,
            status_channel,
            runtime_handle,
            da_provider,
            log_provider,
            proof_provider,
        );
        let builder = ServiceBuilder::<OLCheckpointService, _>::new()
            .with_state(state)
            .with_input(input);

        let monitor = builder
            .launch_sync("ol_checkpoint", executor)
            .map_err(|e| {
                OLCheckpointError::Unexpected(format!("failed to launch service: {}", e))
            })?;

        Ok(OLCheckpointHandle::new(monitor))
    }
}

impl Default for OLCheckpointBuilder {
    fn default() -> Self {
        Self::new()
    }
}
