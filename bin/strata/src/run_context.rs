//! Runtime context holding handles to running services.

use std::sync::Arc;

use strata_asm_params::AsmParams;
use strata_asm_worker::AsmWorkerHandle;
#[cfg(feature = "sequencer")]
use strata_btcio::{broadcaster::L1BroadcastHandle, writer::EnvelopeHandle};
use strata_chain_worker_new::ChainWorkerHandle;
use strata_config::Config;
use strata_consensus_logic::{FcmServiceHandle, SyncServiceHandle};
use strata_csm_worker::CsmWorkerStatus;
use strata_node_context::{CommonContext, NodeContext};
#[cfg(feature = "sequencer")]
use strata_ol_block_assembly::BlockasmHandle;
use strata_ol_checkpoint::OLCheckpointWorkerHandle;
use strata_ol_mempool::MempoolHandle;
#[cfg(feature = "sequencer")]
use strata_service::DumbTickHandle;
use strata_service::ServiceMonitor;
use strata_status::StatusChannel;
use strata_storage::NodeStorage;
use strata_tasks::{TaskExecutor, TaskManager};

/// Holds handles and monitors for all running services.
pub(crate) struct RunContext {
    pub task_manager: TaskManager,
    common: CommonContext,
    service_handles: ServiceHandles,
}

impl RunContext {
    /// Creates a new [`RunContext`] instance from a [`NodeContext`] and [`ServiceHandles`].
    pub(crate) fn from_node_ctx(ctx: NodeContext, service_handles: ServiceHandles) -> Self {
        let (task_manager, common) = ctx.into_parts();
        Self {
            task_manager,
            common,
            service_handles,
        }
    }

    /// Returns the config.
    pub(crate) fn config(&self) -> &Config {
        self.common.config()
    }

    pub(crate) fn asm_params(&self) -> &Arc<AsmParams> {
        self.common.asm_params()
    }

    pub(crate) fn params(&self) -> &Arc<strata_params::Params> {
        self.common.params()
    }

    #[cfg(feature = "prover")]
    pub(crate) fn ol_params(&self) -> &Arc<strata_ol_params::OLParams> {
        self.common.ol_params()
    }

    /// Returns the storage.
    pub(crate) fn storage(&self) -> &Arc<NodeStorage> {
        self.common.storage()
    }

    /// Returns the status channel.
    pub(crate) fn status_channel(&self) -> &Arc<StatusChannel> {
        self.common.status_channel()
    }

    /// Returns the mempool handle if this node runs the mempool.
    pub(crate) fn mempool_handle(&self) -> Option<&Arc<MempoolHandle>> {
        self.service_handles.mempool_handle.as_ref()
    }

    /// Returns the chain worker handle.
    #[cfg(feature = "prover")]
    pub(crate) fn chain_worker_handle(&self) -> &Arc<ChainWorkerHandle> {
        &self.service_handles.chain_worker_handle
    }

    /// Returns the fork-choice manager handle if this node runs FCM.
    pub(crate) fn fcm_handle(&self) -> Option<&Arc<FcmServiceHandle>> {
        match &self.service_handles.sync_handle {
            SyncServiceHandle::Fcm(handle) => Some(handle),
            SyncServiceHandle::Css(_) => None,
        }
    }

    /// Returns the executor.
    pub(crate) fn executor(&self) -> &Arc<TaskExecutor> {
        self.common.executor()
    }

    /// Returns the task manager.
    #[cfg(feature = "sequencer")]
    pub(crate) fn task_manager(&self) -> &TaskManager {
        &self.task_manager
    }

    /// Returns the sequencer handles if running as a sequencer.
    #[cfg(feature = "sequencer")]
    pub(crate) fn sequencer_handles(&self) -> Option<&SequencerServiceHandles> {
        self.service_handles.sequencer_handles.as_ref()
    }
}

/// Sequencer-specific service handles.
///
/// Groups handles for services that only run on sequencer node: L1 broadcast,
/// envelope signing, and block assembly. Stored as `Option` in
/// [`ServiceHandles`] since fullnodes don't run these services.
#[cfg(feature = "sequencer")]
pub(crate) struct SequencerServiceHandles {
    /// Handle for broadcasting L1 transactions using [`strata_btcio`].
    #[expect(unused, reason = "will be used")]
    broadcast_handle: Arc<L1BroadcastHandle>,

    /// Handle for submitting on-chain transactions using [`strata_btcio`].
    envelope_handle: Arc<EnvelopeHandle>,

    /// Handle for the block assembly service.
    blockasm_handle: Arc<BlockasmHandle>,

    /// Held so the L1 watcher service stops when this struct is dropped.
    _watcher_shutdown_guard: DumbTickHandle,
}

#[cfg(feature = "sequencer")]
impl SequencerServiceHandles {
    /// Creates a new [`SequencerServiceHandles`] instance.
    pub(crate) fn new(
        broadcast_handle: Arc<L1BroadcastHandle>,
        envelope_handle: Arc<EnvelopeHandle>,
        blockasm_handle: Arc<BlockasmHandle>,
        watcher_shutdown_guard: DumbTickHandle,
    ) -> Self {
        Self {
            broadcast_handle,
            envelope_handle,
            blockasm_handle,
            _watcher_shutdown_guard: watcher_shutdown_guard,
        }
    }

    /// Returns the envelope handle for submitting on-chain transactions using [`strata_btcio`].
    pub(crate) fn envelope_handle(&self) -> &Arc<EnvelopeHandle> {
        &self.envelope_handle
    }

    /// Returns the block assembly handle.
    pub(crate) fn blockasm_handle(&self) -> &Arc<BlockasmHandle> {
        &self.blockasm_handle
    }
}

/// Handles for all services.
#[expect(unused, reason = "will be used later")]
pub(crate) struct ServiceHandles {
    /// Handle for the ASM worker.
    asm_handle: Arc<AsmWorkerHandle>,

    /// Handle for the CSM worker.
    csm_monitor: Arc<ServiceMonitor<CsmWorkerStatus>>,

    /// Handle for the mempool ([`None`] on checkpoint-sync nodes).
    mempool_handle: Option<Arc<MempoolHandle>>,

    /// Handle for the chain worker.
    chain_worker_handle: Arc<ChainWorkerHandle>,

    /// Handle for the checkpoint worker ([`None`] on checkpoint-sync nodes
    /// which don't author L1 checkpoints).
    checkpoint_handle: Option<Arc<OLCheckpointWorkerHandle>>,

    /// Handle for the OL sync service (FCM or checkpoint sync).
    sync_handle: SyncServiceHandle,

    /// Handles for sequencer-specific services ([`None`] when not running as sequencer).
    #[cfg(feature = "sequencer")]
    sequencer_handles: Option<SequencerServiceHandles>,
}

impl ServiceHandles {
    /// Creates a new [`ServiceHandlesBuilder`] with required handles.
    pub(crate) fn builder(
        asm_handle: Arc<AsmWorkerHandle>,
        csm_monitor: Arc<ServiceMonitor<CsmWorkerStatus>>,
        mempool_handle: Option<Arc<MempoolHandle>>,
        chain_worker_handle: Arc<ChainWorkerHandle>,
        checkpoint_handle: Option<Arc<OLCheckpointWorkerHandle>>,
        sync_handle: SyncServiceHandle,
    ) -> ServiceHandlesBuilder {
        ServiceHandlesBuilder {
            asm_handle,
            csm_monitor,
            mempool_handle,
            chain_worker_handle,
            checkpoint_handle,
            sync_handle,
            #[cfg(feature = "sequencer")]
            sequencer_handles: None,
        }
    }
}

/// Builder for [`ServiceHandles`].
pub(crate) struct ServiceHandlesBuilder {
    /// Handle for the ASM worker.
    asm_handle: Arc<AsmWorkerHandle>,

    /// Handle for the CSM worker.
    csm_monitor: Arc<ServiceMonitor<CsmWorkerStatus>>,

    /// Handle for the mempool ([`None`] on checkpoint-sync nodes).
    mempool_handle: Option<Arc<MempoolHandle>>,

    /// Handle for the chain worker.
    chain_worker_handle: Arc<ChainWorkerHandle>,

    /// Handle for the checkpoint worker ([`None`] on checkpoint-sync nodes).
    checkpoint_handle: Option<Arc<OLCheckpointWorkerHandle>>,

    /// Handle for the OL sync service (FCM or checkpoint sync).
    sync_handle: SyncServiceHandle,

    /// Handles for sequencer-specific services ([`None`] when not running as sequencer).
    #[cfg(feature = "sequencer")]
    sequencer_handles: Option<SequencerServiceHandles>,
}

impl ServiceHandlesBuilder {
    /// Adds sequencer-specific handles.
    #[cfg(feature = "sequencer")]
    pub(crate) fn with_sequencer_handles(
        mut self,
        sequencer_handles: Option<SequencerServiceHandles>,
    ) -> Self {
        self.sequencer_handles = sequencer_handles;
        self
    }

    /// Builds [`ServiceHandles`].
    pub(crate) fn build(self) -> ServiceHandles {
        ServiceHandles {
            asm_handle: self.asm_handle,
            csm_monitor: self.csm_monitor,
            mempool_handle: self.mempool_handle,
            chain_worker_handle: self.chain_worker_handle,
            checkpoint_handle: self.checkpoint_handle,
            sync_handle: self.sync_handle,
            #[cfg(feature = "sequencer")]
            sequencer_handles: self.sequencer_handles,
        }
    }
}
