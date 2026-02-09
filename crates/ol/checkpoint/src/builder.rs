//! Builder pattern for launching the OL checkpoint service.

use std::sync::Arc;

use anyhow::Context;
use strata_node_context::NodeContext;
use strata_primitives::epoch::EpochCommitment;
use strata_service::{ServiceBuilder, SyncAsyncInput, TokioWatchInput};
use strata_storage::NodeStorage;
use strata_tasks::TaskExecutor;
use tokio::sync::watch;

use crate::{
    context::CheckpointWorkerContextImpl, handle::OLCheckpointWorkerHandle,
    service::OLCheckpointService, state::OLCheckpointServiceState,
};

#[expect(
    missing_debug_implementations,
    reason = "Some inner types don't have Debug implementation"
)]
/// Builder for constructing and launching an OL checkpoint service.
pub struct OLCheckpointBuilder {
    storage: Option<Arc<NodeStorage>>,
    epoch_summary_rx: Option<watch::Receiver<Option<EpochCommitment>>>,
}

impl OLCheckpointBuilder {
    /// Create a new builder instance.
    pub fn new() -> Self {
        Self {
            storage: None,
            epoch_summary_rx: None,
        }
    }

    /// Set storage from [`NodeContext`].
    pub fn with_node_context(mut self, nodectx: &NodeContext) -> Self {
        self.storage = Some(nodectx.storage().clone());
        self
    }

    /// Set the epoch summary receiver for driving checkpoint creation.
    pub fn with_epoch_summary_receiver(
        mut self,
        receiver: watch::Receiver<Option<EpochCommitment>>,
    ) -> Self {
        self.epoch_summary_rx = Some(receiver);
        self
    }

    /// Launch the OL checkpoint service and return a handle to it.
    pub fn launch(self, executor: &TaskExecutor) -> anyhow::Result<OLCheckpointWorkerHandle> {
        let storage = self
            .storage
            .context("missing required dependency: storage")?;
        let epoch_summary_rx = self
            .epoch_summary_rx
            .context("missing required dependency: epoch_summary_rx")?;

        let runtime_handle = executor.handle().clone();
        let input = TokioWatchInput::from_receiver(epoch_summary_rx);
        let input = SyncAsyncInput::new(input, runtime_handle);

        let ctx = CheckpointWorkerContextImpl::new(storage);
        let state = OLCheckpointServiceState::new(ctx);
        let builder = ServiceBuilder::<OLCheckpointService<CheckpointWorkerContextImpl>, _>::new()
            .with_state(state)
            .with_input(input);

        let monitor = builder
            .launch_sync("ol_checkpoint", executor)
            .map_err(|e| anyhow::anyhow!("failed to launch service: {}", e))?;

        Ok(OLCheckpointWorkerHandle::new(monitor))
    }
}

impl Default for OLCheckpointBuilder {
    fn default() -> Self {
        Self::new()
    }
}
