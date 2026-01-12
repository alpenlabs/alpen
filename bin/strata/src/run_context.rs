//! Runtime context holding handles to running services.

use std::sync::Arc;

use strata_asm_worker::AsmWorkerHandle;
use strata_chain_worker_new::ChainWorkerHandle;
use strata_config::Config;
use strata_csm_worker::CsmWorkerStatus;
use strata_ol_mempool::MempoolHandle;
use strata_params::Params;
use strata_service::ServiceMonitor;
use strata_status::StatusChannel;
use strata_storage::NodeStorage;
use strata_tasks::{TaskExecutor, TaskManager};
use tokio::runtime::Runtime;

/// Holds handles and monitors for all running services.
#[expect(unused, reason = "will be used later")]
pub(crate) struct RunContext {
    // Common items.
    pub runtime: Runtime,
    pub executor: TaskExecutor,
    pub task_manager: TaskManager,
    pub params: Arc<Params>,
    pub config: Config,
    // Service handles
    pub asm_handle: AsmWorkerHandle,
    pub csm_monitor: ServiceMonitor<CsmWorkerStatus>,
    pub mempool_handle: MempoolHandle,
    pub chain_worker_handle: ChainWorkerHandle,
    // Shared infrastructure
    pub storage: Arc<NodeStorage>,
    pub status_channel: Arc<StatusChannel>,
}
