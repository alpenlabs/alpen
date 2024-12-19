//! Bootstraps an RPC server for the prover client.

use anyhow::{Context, Ok};
use async_trait::async_trait;
use jsonrpsee::{core::RpcResult, RpcModule};
use strata_prover_client_rpc_api::StrataProverClientApiServer;
use strata_rpc_types::RpcCheckpointInfo;
use tokio::sync::oneshot;
use tracing::{info, warn};
use uuid::Uuid;

use crate::{
    dispatcher::TaskDispatcher,
    proving_ops::{
        btc_ops::BtcOperations,
        checkpoint_ops::{CheckpointOperations, CheckpointOpsParam},
        cl_ops::ClOperations,
        el_ops::ElOperations,
        l1_batch_ops::L1BatchOperations,
        l2_batch_ops::L2BatchOperations,
    },
};

#[derive(Clone)]
pub struct RpcContext {
    pub btc_proving_task_dispatcher: TaskDispatcher<BtcOperations>,
    pub el_proving_task_dispatcher: TaskDispatcher<ElOperations>,
    pub cl_proving_task_dispatcher: TaskDispatcher<ClOperations>,
    pub l1_batch_task_dispatcher: TaskDispatcher<L1BatchOperations>,
    pub l2_batch_task_dispatcher: TaskDispatcher<L2BatchOperations>,
    pub checkpoint_dispatcher: TaskDispatcher<CheckpointOperations>,
}

impl RpcContext {
    pub fn new(
        btc_proving_task_scheduler: TaskDispatcher<BtcOperations>,
        el_proving_task_scheduler: TaskDispatcher<ElOperations>,
        cl_proving_task_scheduler: TaskDispatcher<ClOperations>,
        l1_batch_task_scheduler: TaskDispatcher<L1BatchOperations>,
        l2_batch_task_scheduler: TaskDispatcher<L2BatchOperations>,
        checkpoint_scheduler: TaskDispatcher<CheckpointOperations>,
    ) -> Self {
        Self {
            btc_proving_task_dispatcher: btc_proving_task_scheduler,
            el_proving_task_dispatcher: el_proving_task_scheduler,
            cl_proving_task_dispatcher: cl_proving_task_scheduler,
            l1_batch_task_dispatcher: l1_batch_task_scheduler,
            l2_batch_task_dispatcher: l2_batch_task_scheduler,
            checkpoint_dispatcher: checkpoint_scheduler,
        }
    }
}

pub(crate) async fn start<T>(
    rpc_impl: &T,
    rpc_url: String,
    enable_dev_rpc: bool,
) -> anyhow::Result<()>
where
    T: StrataProverClientApiServer + Clone,
{
    let mut rpc_module = RpcModule::new(rpc_impl.clone());

    if enable_dev_rpc {
        let prover_client_dev_api = StrataProverClientApiServer::into_rpc(rpc_impl.clone());
        rpc_module
            .merge(prover_client_dev_api)
            .context("merge prover client api")?;
    }

    info!("connecting to the server {:?}", rpc_url);
    let rpc_server = jsonrpsee::server::ServerBuilder::new()
        .build(&rpc_url)
        .await
        .expect("build prover rpc server");

    let rpc_handle = rpc_server.start(rpc_module);
    let (_stop_tx, stop_rx): (oneshot::Sender<bool>, oneshot::Receiver<bool>) = oneshot::channel();
    info!("prover client  RPC server started at: {}", rpc_url);

    let _ = stop_rx.await;
    info!("stopping RPC server");

    if rpc_handle.stop().is_err() {
        warn!("rpc server already stopped");
    }

    Ok(())
}

/// Struct to implement the `strata_prover_client_rpc_api::StrataProverClientApiServer` on.
/// Contains fields corresponding the global context for the RPC.
#[derive(Clone)]
pub(crate) struct ProverClientRpc {
    context: RpcContext,
}

impl ProverClientRpc {
    pub fn new(context: RpcContext) -> Self {
        Self { context }
    }
}

#[async_trait]
impl StrataProverClientApiServer for ProverClientRpc {
    async fn prove_btc_block(&self, btc_block_num: u64) -> RpcResult<Uuid> {
        let task_id = self
            .context
            .btc_proving_task_dispatcher
            .create_task(btc_block_num)
            .await
            .expect("failed to add proving task, l1 block");

        RpcResult::Ok(task_id)
    }

    async fn prove_el_blocks(&self, block_range: (u64, u64)) -> RpcResult<Uuid> {
        let task_id = self
            .context
            .el_proving_task_dispatcher
            .create_task(block_range)
            .await
            .expect("failed to add proving task, el block");

        RpcResult::Ok(task_id)
    }

    async fn prove_cl_block(&self, cl_block_num: u64) -> RpcResult<Uuid> {
        let task_id = self
            .context
            .cl_proving_task_dispatcher
            .create_task(cl_block_num)
            .await
            .expect("failed to add proving task, cl block");

        RpcResult::Ok(task_id)
    }

    async fn prove_l1_batch(&self, l1_range: (u64, u64)) -> RpcResult<Uuid> {
        let task_id = self
            .context
            .l1_batch_task_dispatcher
            .create_task(l1_range)
            .await
            .expect("failed to add proving task, l1 batch");

        RpcResult::Ok(task_id)
    }

    async fn prove_l2_batch(&self, l2_range: (u64, u64)) -> RpcResult<Uuid> {
        let task_id = self
            .context
            .l2_batch_task_dispatcher
            .create_task(l2_range)
            .await
            .expect("failed to add proving task, cl batch");

        RpcResult::Ok(task_id)
    }

    async fn prove_latest_checkpoint(&self) -> RpcResult<Uuid> {
        let task_id = self
            .context
            .checkpoint_dispatcher
            .create_task(CheckpointOpsParam::Latest)
            .await
            .expect("failed to add proving task, checkpoint");

        RpcResult::Ok(task_id)
    }

    async fn prove_checkpoint_raw(
        &self,
        checkpoint_idx: u64,
        l1_range: (u64, u64),
        l2_range: (u64, u64),
    ) -> RpcResult<Uuid> {
        let checkpoint_info = RpcCheckpointInfo {
            idx: checkpoint_idx,
            l1_range,
            l2_range,
            l2_blockid: Default::default(),
        };

        let task_id = self
            .context
            .checkpoint_dispatcher
            .create_task(CheckpointOpsParam::Manual(checkpoint_info))
            .await
            .expect("failed to add proving task, checkpoint");

        RpcResult::Ok(task_id)
    }

    async fn get_task_status(&self, task_id: Uuid) -> RpcResult<Option<String>> {
        let task_tracker = self.context.el_proving_task_dispatcher.task_tracker();

        if let Some(task_status) = task_tracker.get_task_status(task_id).await {
            return RpcResult::Ok(Some(task_status.to_string()));
        }

        RpcResult::Ok(None)
    }
}
