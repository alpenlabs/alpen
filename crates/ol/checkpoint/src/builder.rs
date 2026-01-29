//! Builder pattern for launching the OL checkpoint service.

use std::sync::Arc;

use strata_service::ServiceBuilder;
use strata_status::StatusChannel;
use strata_storage::NodeStorage;
use strata_tasks::TaskExecutor;
use tokio::runtime::Handle;

use crate::{
    errors::{OLCheckpointError, WorkerResult},
    handle::OLCheckpointHandle,
    providers::{
        DaProvider, EmptyLogProvider, FullStateDaProvider, LogProvider, PlaceholderProofProvider,
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
            da_provider: None,
            log_provider: None,
            proof_provider: None,
        }
    }

    /// Set the storage handle.
    pub fn with_storage(mut self, storage: Arc<NodeStorage>) -> Self {
        self.storage = Some(storage);
        self
    }

    /// Set the status channel for genesis waiting.
    pub fn with_status_channel(mut self, channel: StatusChannel) -> Self {
        self.status_channel = Some(channel);
        self
    }

    /// Set the runtime handle for blocking operations.
    pub fn with_runtime(mut self, handle: Handle) -> Self {
        self.runtime_handle = Some(handle);
        self
    }

    /// Set a custom DA provider. Defaults to [`FullStateDaProvider`].
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

        // Use v1 defaults if no custom providers specified
        let da_provider = self
            .da_provider
            .unwrap_or_else(|| Arc::new(FullStateDaProvider::new(storage.ol_state().clone())));
        let log_provider = self
            .log_provider
            .unwrap_or_else(|| Arc::new(EmptyLogProvider));
        let proof_provider = self
            .proof_provider
            .unwrap_or_else(|| Arc::new(PlaceholderProofProvider));

        let state = OLCheckpointServiceState::new(
            storage,
            status_channel,
            runtime_handle,
            da_provider,
            log_provider,
            proof_provider,
        );
        let mut builder = ServiceBuilder::<OLCheckpointService, _>::new().with_state(state);
        let command_handle = builder.create_command_handle(64);

        builder
            .launch_sync("ol_checkpoint", executor)
            .map_err(|e| {
                OLCheckpointError::Unexpected(format!("failed to launch service: {}", e))
            })?;

        Ok(OLCheckpointHandle::new(command_handle))
    }
}

impl Default for OLCheckpointBuilder {
    fn default() -> Self {
        Self::new()
    }
}
