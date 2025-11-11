//! Bootstraps an RPC server for the prover client.

use std::{collections::HashMap, sync::Arc};

use anyhow::Context;
use async_trait::async_trait;
use jsonrpsee::{core::RpcResult, RpcModule};
use strata_db_store_sled::prover::ProofDBSled;
use strata_db_types::traits::ProofDatabase;
use strata_paas::{ProverHandle, ZkVmBackend, ZkVmTaskId};
use strata_primitives::{
    evm_exec::EvmEeBlockCommitment, l1::L1BlockCommitment, l2::L2BlockCommitment, proof::{ProofContext, ProofKey},
};
use strata_prover_client_rpc_api::StrataProverClientApiServer;
use strata_rpc_api::StrataApiClient;
use strata_rpc_types::ProofKey as RpcProofKey;
use strata_rpc_utils::to_jsonrpsee_error;
use tokio::sync::oneshot;
use tracing::{info, warn};
use zkaleido::ProofReceipt;

use crate::operators::ProofOperator;

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
    prover_handle: ProverHandle<ProofContext>,
    operator: Arc<ProofOperator>,
    db: Arc<ProofDBSled>,
}

impl ProverClientRpc {
    pub(crate) fn new(
        prover_handle: ProverHandle<ProofContext>,
        operator: Arc<ProofOperator>,
        db: Arc<ProofDBSled>,
    ) -> Self {
        Self {
            prover_handle,
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

    /// Helper to determine backend based on features
    fn get_backend() -> ZkVmBackend {
        #[cfg(feature = "sp1")]
        {
            ZkVmBackend::SP1
        }
        #[cfg(not(feature = "sp1"))]
        {
            ZkVmBackend::Native
        }
    }

    /// Helper to submit a proof context as a task
    async fn submit_proof_context(&self, proof_ctx: ProofContext) -> Result<ProofKey, anyhow::Error> {
        let backend = Self::get_backend();
        let zkvm = match backend {
            ZkVmBackend::SP1 => strata_primitives::proof::ProofZkVm::SP1,
            ZkVmBackend::Native => strata_primitives::proof::ProofZkVm::Native,
            ZkVmBackend::Risc0 => anyhow::bail!("Risc0 not supported"),
        };

        let proof_key = ProofKey::new(proof_ctx, zkvm);

        // Check if proof already exists
        if self.db.get_proof(&proof_key).map_err(|e| anyhow::anyhow!("DB error: {}", e))?.is_some() {
            return Ok(proof_key);
        }

        // Submit task to PaaS
        let task_id = ZkVmTaskId {
            program: proof_ctx,
            backend,
        };

        self.prover_handle
            .submit_task(task_id)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to submit task: {}", e))?;

        Ok(proof_key)
    }

    /// Helper to create tasks from proof context (handles dependencies)
    async fn create_tasks_from_context(&self, proof_ctx: ProofContext) -> Result<Vec<ProofKey>, anyhow::Error> {
        // Get or create proof dependencies
        let proof_deps = self.db
            .get_proof_deps(proof_ctx)
            .map_err(|e| anyhow::anyhow!("DB error: {}", e))?
            .unwrap_or_default();

        // Submit dependency tasks first
        for dep_ctx in &proof_deps {
            self.submit_proof_context(*dep_ctx).await?;
        }

        // Submit main task
        let proof_key = self.submit_proof_context(proof_ctx).await?;

        Ok(vec![proof_key])
    }
}

#[async_trait]
impl StrataProverClientApiServer for ProverClientRpc {
    async fn prove_el_blocks(
        &self,
        el_block_range: (EvmEeBlockCommitment, EvmEeBlockCommitment),
    ) -> RpcResult<Vec<RpcProofKey>> {
        let proof_ctx = ProofContext::EvmEeStf(el_block_range.0, el_block_range.1);

        self.create_tasks_from_context(proof_ctx)
            .await
            .map_err(to_jsonrpsee_error("failed to create task for el block"))
    }

    async fn prove_cl_blocks(
        &self,
        cl_block_range: (L2BlockCommitment, L2BlockCommitment),
    ) -> RpcResult<Vec<RpcProofKey>> {
        let proof_ctx = ProofContext::ClStf(cl_block_range.0, cl_block_range.1);

        self.create_tasks_from_context(proof_ctx)
            .await
            .map_err(to_jsonrpsee_error("failed to create task for cl block"))
    }

    async fn prove_checkpoint(&self, ckp_idx: u64) -> RpcResult<Vec<RpcProofKey>> {
        let proof_ctx = ProofContext::Checkpoint(ckp_idx);

        self.create_tasks_from_context(proof_ctx)
            .await
            .map_err(to_jsonrpsee_error(
                "failed to create task for given checkpoint",
            ))
    }

    async fn prove_latest_checkpoint(&self) -> RpcResult<Vec<RpcProofKey>> {
        let next_unproven_idx = self
            .operator
            .checkpoint_operator()
            .cl_client()
            .get_next_unproven_checkpoint_index()
            .await
            .map_err(to_jsonrpsee_error(
                "failed to fetch next unproven checkpoint idx",
            ))?;

        let checkpoint_idx = match next_unproven_idx {
            Some(idx) => {
                info!(unproven_checkpoint = %idx, "proving next unproven checkpoint");
                idx
            }
            None => {
                info!("no unproven checkpoints found");
                return Ok(vec![]);
            }
        };

        let proof_ctx = ProofContext::Checkpoint(checkpoint_idx);

        self.create_tasks_from_context(proof_ctx)
            .await
            .map_err(to_jsonrpsee_error(
                "failed to create task for next unproven checkpoint",
            ))
    }

    async fn prove_checkpoint_raw(
        &self,
        checkpoint_idx: u64,
        _l1_range: (L1BlockCommitment, L1BlockCommitment),
        _l2_range: (L2BlockCommitment, L2BlockCommitment),
    ) -> RpcResult<Vec<RpcProofKey>> {
        // For raw checkpoint, we just create the checkpoint proof context
        // The dependencies are handled by the operators when fetching input
        let proof_ctx = ProofContext::Checkpoint(checkpoint_idx);

        self.create_tasks_from_context(proof_ctx)
            .await
            .map_err(to_jsonrpsee_error(
                "failed to create task for raw checkpoint",
            ))
    }

    async fn get_task_status(&self, key: RpcProofKey) -> RpcResult<String> {
        // First check in DB if the proof is already present
        let proof = self
            .db
            .get_proof(&key)
            .map_err(to_jsonrpsee_error("db failure"))?;

        match proof {
            // If proof is in DB, it was completed
            Some(_) => Ok("Completed".to_string()),
            // If proof is not in DB, check PaaS status
            None => {
                let backend = Self::get_backend();
                let task_id = ZkVmTaskId {
                    program: *key.context(),
                    backend,
                };

                let status = self
                    .prover_handle
                    .get_status(&task_id)
                    .await
                    .map_err(to_jsonrpsee_error("failed to get task status"))?;

                Ok(format!("{:?}", status))
            }
        }
    }

    async fn get_proof(&self, key: RpcProofKey) -> RpcResult<Option<ProofReceipt>> {
        let proof = self
            .db
            .get_proof(&key)
            .map_err(to_jsonrpsee_error("proof not found in db"))?;

        Ok(proof.map(|p| p.receipt().clone()))
    }

    async fn get_report(&self) -> RpcResult<HashMap<String, usize>> {
        let summary = self
            .prover_handle
            .get_summary()
            .await
            .map_err(to_jsonrpsee_error("failed to get summary"))?;

        let mut report = HashMap::new();
        report.insert("total".to_string(), summary.total);
        report.insert("pending".to_string(), summary.pending);
        report.insert("queued".to_string(), summary.queued);
        report.insert("proving".to_string(), summary.proving);
        report.insert("completed".to_string(), summary.completed);
        report.insert("transient_failure".to_string(), summary.transient_failure);
        report.insert("permanent_failure".to_string(), summary.permanent_failure);

        Ok(report)
    }
}
