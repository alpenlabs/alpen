//! Builder pattern for launching the OL checkpoint service.

use std::sync::Arc;

use anyhow::Context;
use strata_bridge_params::BridgeParams;
use strata_identifiers::L1BlockCommitment;
use strata_node_context::NodeContext;
use strata_ol_params::OLRuntimeParams;
use strata_service::{ServiceBuilder, SyncAsyncInput, TokioWatchInput};
use strata_status::OLSyncStatusUpdate;
use strata_storage::NodeStorage;
use strata_tasks::TaskExecutor;
use tokio::sync::watch;

use crate::ProverConfig;
use crate::context::CheckpointWorkerContextImpl;
use crate::handle::OLCheckpointWorkerHandle;
use crate::service::OLCheckpointService;
use crate::state::OLCheckpointServiceState;

#[expect(
    missing_debug_implementations,
    reason = "Some inner types don't have Debug implementation"
)]
/// Builder for constructing and launching an OL checkpoint service.
pub struct OLCheckpointBuilder {
    storage: Option<Arc<NodeStorage>>,
    runtime_params: Option<OLRuntimeParams>,
    genesis_l1_block: Option<L1BlockCommitment>,
    ol_sync_rx: Option<watch::Receiver<Option<OLSyncStatusUpdate>>>,
    prover: Option<ProverConfig>,
}

impl OLCheckpointBuilder {
    /// Create a new builder instance.
    pub fn new() -> Self {
        Self {
            storage: None,
            runtime_params: None,
            genesis_l1_block: None,
            ol_sync_rx: None,
            prover: None,
        }
    }

    /// Set storage, withdrawal params and the genesis L1 anchor from
    /// [`NodeContext`].
    pub fn with_node_context(mut self, nodectx: &NodeContext) -> Self {
        self.storage = Some(nodectx.storage().clone());
        self.runtime_params = Some(nodectx.ol_params().runtime_params());
        self.genesis_l1_block = Some(nodectx.ol_params().genesis_l1_block());
        self
    }

    /// Set the bridge denomination and withdrawal policy.
    pub fn with_bridge_params(mut self, bridge_params: BridgeParams) -> Self {
        self.runtime_params = Some(OLRuntimeParams::new(bridge_params));
        self
    }

    /// Sets the receiver notified after canonical index updates.
    pub fn with_ol_sync_receiver(
        mut self,
        receiver: watch::Receiver<Option<OLSyncStatusUpdate>>,
    ) -> Self {
        self.ol_sync_rx = Some(receiver);
        self
    }

    /// Set the prover configuration.
    ///
    /// When set, the checkpoint worker looks up proofs from the proof DB
    /// and waits while the selected commitment remains canonical.
    /// When absent, empty proofs are used.
    pub fn with_prover(mut self, prover: ProverConfig) -> Self {
        self.prover = Some(prover);
        self
    }

    /// Launch the OL checkpoint service and return a handle to it.
    pub fn launch(self, executor: &TaskExecutor) -> anyhow::Result<OLCheckpointWorkerHandle> {
        let storage = self
            .storage
            .context("missing required dependency: storage")?;
        let ol_sync_rx = self
            .ol_sync_rx
            .context("missing required dependency: ol_sync_rx")?;
        let runtime_params = self
            .runtime_params
            .context("missing required dependency: runtime_params")?;
        let genesis_l1_block = self
            .genesis_l1_block
            .context("missing required dependency: genesis_l1_block")?;

        let runtime_handle = executor.handle().clone();
        let input = TokioWatchInput::from_receiver(ol_sync_rx);
        let input = SyncAsyncInput::new(input, runtime_handle);

        let ctx = match self.prover {
            Some(prover) => CheckpointWorkerContextImpl::with_prover(
                storage,
                runtime_params,
                genesis_l1_block,
                prover,
            ),
            None => CheckpointWorkerContextImpl::new(storage, runtime_params, genesis_l1_block),
        };
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
