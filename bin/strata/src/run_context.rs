//! Runtime context holding handles to running services.

use std::sync::Arc;

use strata_asm_worker::AsmWorkerHandle;
use strata_chain_worker_new::ChainWorkerHandle;
use strata_config::Config;
use strata_csm_worker::CsmWorkerStatus;
use strata_node_context::{CommonContext, NodeContext};
use strata_ol_mempool::MempoolHandle;
use strata_service::ServiceMonitor;
use strata_status::StatusChannel;
use strata_storage::NodeStorage;
use strata_tasks::{TaskExecutor, TaskManager};

/// Holds handles and monitors for all running services.
pub(crate) struct RunContext {
    pub task_manager: TaskManager,
    pub common: CommonContext,
    pub service_handles: ServiceHandles,
}

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

    pub(crate) fn into_manager(self) -> TaskManager {
        self.task_manager
    }
}

#[expect(unused, reason = "will be used later")]
pub(crate) struct ServiceHandles {
    asm_handle: Arc<AsmWorkerHandle>,
    csm_monitor: Arc<ServiceMonitor<CsmWorkerStatus>>,
    mempool_handle: Arc<MempoolHandle>,
    chain_worker_handle: Arc<ChainWorkerHandle>,
}

impl ServiceHandles {
    pub(crate) fn new(
        asm_handle: Arc<AsmWorkerHandle>,
        csm_monitor: Arc<ServiceMonitor<CsmWorkerStatus>>,
        mempool_handle: Arc<MempoolHandle>,
        chain_worker_handle: Arc<ChainWorkerHandle>,
    ) -> Self {
        Self {
            asm_handle,
            csm_monitor,
            mempool_handle,
            chain_worker_handle,
        }
    }
}
