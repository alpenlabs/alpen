//! Runtime context holding handles to running services.

use std::sync::Arc;

use strata_asm_worker::AsmWorkerHandle;
#[cfg(feature = "sequencer")]
use strata_btcio::{broadcaster::L1BroadcastHandle, writer::EnvelopeHandle};
use strata_chain_worker_new::ChainWorkerHandle;
use strata_config::Config;
use strata_consensus_logic::FcmServiceHandle;
use strata_csm_worker::CsmWorkerStatus;
use strata_node_context::{CommonContext, NodeContext};
use strata_ol_block_assembly::BlockasmHandle;
use strata_ol_checkpoint::OLCheckpointWorkerHandle;
use strata_ol_mempool::MempoolHandle;
#[cfg(feature = "sequencer")]
use strata_ol_sequencer::TemplateManager;
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

    /// Returns the storage.
    pub(crate) fn storage(&self) -> &Arc<NodeStorage> {
        self.common.storage()
    }

    /// Returns the status channel.
    pub(crate) fn status_channel(&self) -> &Arc<StatusChannel> {
        self.common.status_channel()
    }

    /// Returns the mempool handle.
    pub(crate) fn mempool_handle(&self) -> &Arc<MempoolHandle> {
        &self.service_handles.mempool_handle
    }

    /// Returns the fork choice manager handle.
    pub(crate) fn fcm_handle(&self) -> &Arc<FcmServiceHandle> {
        &self.service_handles.fcm_handle
    }

    /// Returns the executor.
    pub(crate) fn executor(&self) -> &Arc<TaskExecutor> {
        self.common.executor()
    }

    /// Returns the task manager.
    pub(crate) fn task_manager(&self) -> &TaskManager {
        &self.task_manager
    }

    #[cfg(feature = "sequencer")]
    /// Returns the sequencer handles if running as a sequencer.
    pub(crate) fn sequencer_handles(&self) -> Option<&SequencerServiceHandles> {
        self.service_handles.sequencer_handles.as_ref()
    }
}

/// Sequencer-specific service handles.
///
/// Groups handles for services that only run on sequencer node: L1 broadcast,
/// envelope signing, and block assembly. Stored as `Option` in [`ServiceHandles`]
/// since fullnodes don't run these services.
#[cfg(feature = "sequencer")]
pub(crate) struct SequencerServiceHandles {
    /// Handle for broadcasting L1 transactions using [`strata_btcio`].
    #[expect(unused, reason = "will be used")]
    broadcast_handle: Arc<L1BroadcastHandle>,

    /// Handle for submitting on-chain transactions using [`strata_btcio`].
    envelope_handle: Arc<EnvelopeHandle>,

    /// Handle for managing block templates.
    template_manager: Arc<TemplateManager>,
}

#[cfg(feature = "sequencer")]
impl SequencerServiceHandles {
    /// Creates a new [`SequencerServiceHandles`] instance.
    pub(crate) fn new(
        broadcast_handle: Arc<L1BroadcastHandle>,
        envelope_handle: Arc<EnvelopeHandle>,
        template_manager: Arc<TemplateManager>,
    ) -> Self {
        Self {
            broadcast_handle,
            envelope_handle,
            template_manager,
        }
    }

    /// Returns the envelope handle for submitting on-chain transactions using [`strata_btcio`].
    pub(crate) fn envelope_handle(&self) -> &Arc<EnvelopeHandle> {
        &self.envelope_handle
    }

    /// Returns the template manager handle.
    pub(crate) fn template_manager(&self) -> &Arc<TemplateManager> {
        &self.template_manager
    }
}

/// Handles for all services.
#[expect(unused, reason = "will be used later")]
pub(crate) struct ServiceHandles {
    /// Handle for the ASM worker.
    asm_handle: Arc<AsmWorkerHandle>,

    /// Handle for the CSM worker.
    csm_monitor: Arc<ServiceMonitor<CsmWorkerStatus>>,

    /// Handle for the mempool.
    mempool_handle: Arc<MempoolHandle>,

    /// Handle for the chain worker.
    chain_worker_handle: Arc<ChainWorkerHandle>,

    /// Handle for the checkpoint worker.
    checkpoint_handle: Arc<OLCheckpointWorkerHandle>,

    /// Handle for the FCM service.
    fcm_handle: Arc<FcmServiceHandle>,

    #[cfg(feature = "sequencer")]
    /// Handles for sequencer-specific services ([`None`] when not running as sequencer).
    sequencer_handles: Option<SequencerServiceHandles>,
}

impl ServiceHandles {
    /// Creates a new [`ServiceHandles`] instance.
    pub(crate) fn new(
        asm_handle: Arc<AsmWorkerHandle>,
        csm_monitor: Arc<ServiceMonitor<CsmWorkerStatus>>,
        mempool_handle: Arc<MempoolHandle>,
        chain_worker_handle: Arc<ChainWorkerHandle>,
        checkpoint_handle: Arc<OLCheckpointWorkerHandle>,
        fcm_handle: Arc<FcmServiceHandle>,
        #[cfg(feature = "sequencer")] sequencer_handles: Option<SequencerServiceHandles>,
    ) -> Self {
        Self {
            asm_handle,
            csm_monitor,
            mempool_handle,
            chain_worker_handle,
            checkpoint_handle,
            fcm_handle,
            #[cfg(feature = "sequencer")]
            sequencer_handles,
        }
    }
}
