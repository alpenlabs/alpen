//! Runtime context holding handles to running services.

use std::sync::Arc;

use strata_asm_worker::AsmWorkerHandle;
use strata_btcio::{broadcaster::L1BroadcastHandle, writer::EnvelopeHandle};
use strata_chain_worker_new::ChainWorkerHandle;
use strata_config::Config;
use strata_consensus_logic::FcmServiceHandle;
use strata_csm_worker::CsmWorkerStatus;
use strata_node_context::{CommonContext, NodeContext};
use strata_ol_block_assembly::BlockasmHandle;
use strata_ol_mempool::MempoolHandle;
use strata_params::Params;
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

#[expect(unused, reason = "will be used later")]
impl RunContext {
    pub(crate) fn from_node_ctx(ctx: NodeContext, service_handles: ServiceHandles) -> Self {
        let (task_manager, common) = ctx.into_parts();
        Self {
            task_manager,
            common,
            service_handles,
        }
    }

    pub(crate) fn config(&self) -> &Config {
        self.common.config()
    }

    pub(crate) fn params(&self) -> &Arc<Params> {
        self.common.params()
    }

    pub(crate) fn storage(&self) -> &Arc<NodeStorage> {
        self.common.storage()
    }

    pub(crate) fn status_channel(&self) -> &Arc<StatusChannel> {
        self.common.status_channel()
    }

    pub(crate) fn mempool_handle(&self) -> &Arc<MempoolHandle> {
        &self.service_handles.mempool_handle
    }

    pub(crate) fn executor(&self) -> &Arc<TaskExecutor> {
        self.common.executor()
    }
}

/// Sequencer-specific service handles.
///
/// Groups handles for services that only run on sequencer node: L1 broadcast,
/// envelope signing, and block assembly. Stored as `Option` in [`ServiceHandles`]
/// since fullnodes don't run these services.
#[expect(
    unused, 
    reason = "fields will be accessed when sequencer RPC is implemented"
)]
pub(crate) struct SequencerServiceHandles {
    pub broadcast_handle: Arc<L1BroadcastHandle>,
    pub envelope_handle: Arc<EnvelopeHandle>,
    pub blockasm_handle: BlockasmHandle,
}

#[expect(unused, reason = "will be used later")]
pub(crate) struct ServiceHandles {
    asm_handle: Arc<AsmWorkerHandle>,
    csm_monitor: Arc<ServiceMonitor<CsmWorkerStatus>>,
    mempool_handle: Arc<MempoolHandle>,
    chain_worker_handle: Arc<ChainWorkerHandle>,
    fcm_handle: Arc<FcmServiceHandle>,
    /// Sequencer-specific handles (None for fullnodes)
    sequencer_handles: Option<SequencerServiceHandles>,
}

impl ServiceHandles {
    pub(crate) fn new(
        asm_handle: Arc<AsmWorkerHandle>,
        csm_monitor: Arc<ServiceMonitor<CsmWorkerStatus>>,
        mempool_handle: Arc<MempoolHandle>,
        chain_worker_handle: Arc<ChainWorkerHandle>,
        fcm_handle: Arc<FcmServiceHandle>,
        sequencer_handles: Option<SequencerServiceHandles>,
    ) -> Self {
        Self {
            asm_handle,
            csm_monitor,
            mempool_handle,
            chain_worker_handle,
            fcm_handle,
            sequencer_handles,
        }
    }
}
