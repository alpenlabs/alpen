//! Bootstraps an RPC server for the prover client.

use std::{collections::HashMap, sync::Arc};

use anyhow::Context;
use async_trait::async_trait;
use jsonrpsee::{core::RpcResult, RpcModule};
use strata_db::traits::ProofDatabase;
use strata_db_store_sled::prover::ProofDBSled;
use strata_primitives::{
    evm_exec::EvmEeBlockCommitment, l1::L1BlockCommitment, l2::L2BlockCommitment,
};
use strata_prover_client_rpc_api::StrataProverClientApiServer;
use strata_rpc_types::ProofKey;
use strata_rpc_utils::to_jsonrpsee_error;
use tokio::sync::{oneshot, Mutex};
use tracing::{info, warn};
use zkaleido::ProofReceipt;

use crate::{
    operators::{cl_stf::ClStfParams, ProofOperator, ProvingOp},
    status::ProvingTaskStatus,
    task_tracker::TaskTracker,
};

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
    task_tracker: Arc<Mutex<TaskTracker>>,
    operator: Arc<ProofOperator>,
    db: Arc<ProofDBSled>,
}

impl ProverClientRpc {
    pub(crate) fn new(
        task_tracker: Arc<Mutex<TaskTracker>>,
        operator: Arc<ProofOperator>,
        db: Arc<ProofDBSled>,
    ) -> Self {
        Self {
            task_tracker,
            operator,
            db,
        }
    }

    /// Start the RPC server with the given URL and dev RPC enablement
    pub(crate) async fn start_server(
        &self,
        rpc_url: String,
        enable_dev_rpc: bool,
    ) -> anyhow::Result<()> {
        start(self, rpc_url, enable_dev_rpc).await
    }
}

#[async_trait]
impl StrataProverClientApiServer for ProverClientRpc {
    async fn prove_el_blocks(
        &self,
        el_block_range: (EvmEeBlockCommitment, EvmEeBlockCommitment),
    ) -> RpcResult<Vec<ProofKey>> {
        self.operator
            .evm_ee_operator()
            .create_task(el_block_range, self.task_tracker.clone(), &self.db)
            .await
            .map_err(to_jsonrpsee_error("failed to create task for el block"))
    }

    async fn prove_cl_blocks(
        &self,
        cl_block_range: (L2BlockCommitment, L2BlockCommitment),
    ) -> RpcResult<Vec<ProofKey>> {
        let cl_params = ClStfParams {
            l2_range: cl_block_range,
        };
        self.operator
            .cl_stf_operator()
            .create_task(cl_params, self.task_tracker.clone(), &self.db)
            .await
            .map_err(to_jsonrpsee_error("failed to create task for cl block"))
    }

    async fn prove_checkpoint(&self, ckp_idx: u64) -> RpcResult<Vec<ProofKey>> {
        self.operator
            .checkpoint_operator()
            .create_task(ckp_idx, self.task_tracker.clone(), &self.db)
            .await
            .map_err(to_jsonrpsee_error(
                "failed to create task for given checkpoint",
            ))
    }

    async fn prove_latest_checkpoint(&self) -> RpcResult<Vec<ProofKey>> {
        let latest_unproven_idx = self
            .operator
            .checkpoint_operator()
            .cl_client()
            .get_latest_unproven_checkpoint_index()
            .await
            .map_err(to_jsonrpsee_error(
                "failed to fetch latest unproven checkpoint idx",
            ))?;

        let checkpoint_idx = match latest_unproven_idx {
            Some(idx) => {
                info!(unproven_checkpoint = %idx, "proving latest unproven checkpoint");
                idx
            }
            None => {
                info!("no unproven checkpoints found");
                return Ok(vec![]);
            }
        };

        self.operator
            .checkpoint_operator()
            .create_task(checkpoint_idx, self.task_tracker.clone(), &self.db)
            .await
            .inspect_err(|e| tracing::error!(%e, "prover client Error"))
            .map_err(to_jsonrpsee_error(
                "failed to create task for latest unproven checkpoint",
            ))
    }

    async fn prove_checkpoint_raw(
        &self,
        checkpoint_idx: u64,
        l1_range: (L1BlockCommitment, L1BlockCommitment),
        l2_range: (L2BlockCommitment, L2BlockCommitment),
    ) -> RpcResult<Vec<ProofKey>> {
        self.operator
            .checkpoint_operator()
            .create_task_raw(
                checkpoint_idx,
                l1_range,
                l2_range,
                self.task_tracker.clone(),
                &self.db,
            )
            .await
            .map_err(to_jsonrpsee_error(
                "failed to create task for raw checkpoint",
            ))
    }

    async fn get_task_status(&self, key: ProofKey) -> RpcResult<String> {
        // first check in DB if the proof is already present
        let proof = self
            .db
            .get_proof(&key)
            .map_err(to_jsonrpsee_error("db failure"))?;
        match proof {
            // If proof is in DB, it was completed
            Some(_) => Ok(format!("{:?}", ProvingTaskStatus::Completed)),
            // If proof is in not in DB:
            // - Either the status of the task is in task_tracker
            // - Or the task is invalid
            None => {
                let status = self
                    .task_tracker
                    .lock()
                    .await
                    .get_task(key)
                    .cloned()
                    .map_err(to_jsonrpsee_error("invalid task"))?;
                Ok(format!("{status:?}"))
            }
        }
    }

    async fn get_proof(&self, key: ProofKey) -> RpcResult<Option<ProofReceipt>> {
        let proof = self
            .db
            .get_proof(&key)
            .map_err(to_jsonrpsee_error("proof not found in db"))?;

        Ok(proof.map(|p| p.receipt().clone()))
    }

    async fn get_report(&self) -> RpcResult<HashMap<String, usize>> {
        let task_tracker = self.task_tracker.lock().await;
        Ok(task_tracker.generate_report())
    }
}
