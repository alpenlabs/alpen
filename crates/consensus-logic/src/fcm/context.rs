use std::sync::Arc;

use strata_chain_worker_new::ChainWorkerHandle;
use strata_csm_worker::CsmWorkerStatus;
use strata_ol_node::context::NodeContext;
use strata_params::Params;
use strata_service::ServiceMonitor;
use strata_status::StatusChannel;
use strata_storage::NodeStorage;

#[derive(Clone)]
#[expect(
    missing_debug_implementations,
    reason = "Not all attributes have debug impls"
)]
pub struct FcmContext {
    params: Arc<Params>,
    storage: Arc<NodeStorage>,
    chain_worker: Arc<ChainWorkerHandle>,
    csm_monitor: Arc<ServiceMonitor<CsmWorkerStatus>>,
    status_channel: Arc<StatusChannel>,
}

impl FcmContext {
    pub fn from_node_ctx(
        nodectx: &NodeContext,
        chain_worker: Arc<ChainWorkerHandle>,
        csm_monitor: Arc<ServiceMonitor<CsmWorkerStatus>>,
    ) -> Self {
        Self {
            params: nodectx.params().clone(),
            storage: nodectx.storage().clone(),
            status_channel: nodectx.status_channel().clone(),
            chain_worker,
            csm_monitor,
        }
    }

    pub(crate) fn params(&self) -> &Params {
        &self.params
    }

    pub(crate) fn storage(&self) -> &NodeStorage {
        &self.storage
    }

    pub(crate) fn csm_monitor(&self) -> &ServiceMonitor<CsmWorkerStatus> {
        &self.csm_monitor
    }

    pub(crate) fn status_channel(&self) -> &StatusChannel {
        &self.status_channel
    }

    pub(crate) fn chain_worker(&self) -> Arc<ChainWorkerHandle> {
        self.chain_worker.clone()
    }
}
