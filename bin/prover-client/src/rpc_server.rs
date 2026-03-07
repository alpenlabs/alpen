//! Bootstraps an RPC server for the prover client.

use std::{collections::HashMap, sync::Arc};

use anyhow::Context;
use async_trait::async_trait;
use jsonrpsee::{core::RpcResult, server, RpcModule};
use strata_db_store_sled::prover::ProofDBSled;
use strata_db_types::traits::ProofDatabase;
use strata_paas::{ProverHandle, TaskId};
use strata_primitives::{
    evm_exec::EvmEeBlockCommitment,
    proof::{ProofContext, ProofKey},
};
use strata_prover_client_rpc_api::StrataProverClientApiServer;
use strata_rpc_api::StrataApiClient;
use strata_rpc_types::ProofKey as RpcProofKey;
use strata_rpc_utils::to_jsonrpsee_error;
use tokio::sync::oneshot;
use tracing::{info, warn};
use zkaleido::ProofReceipt;

use crate::{
    operators::CheckpointOperator,
    service::{proof_key_for, zkvm_backend, ProofTask},
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
    let rpc_server = server::ServerBuilder::new()
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
    prover_handle: ProverHandle<ProofTask>,
    checkpoint_operator: CheckpointOperator,
    db: Arc<ProofDBSled>,
}

impl ProverClientRpc {
    pub(crate) fn new(
        prover_handle: ProverHandle<ProofTask>,
        checkpoint_operator: CheckpointOperator,
        db: Arc<ProofDBSled>,
    ) -> Self {
        Self {
            prover_handle,
            checkpoint_operator,
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

    /// Submit a proof context as a task, returning the proof key
    async fn submit_task(&self, proof_ctx: ProofContext) -> Result<ProofKey, anyhow::Error> {
        let proof_key = proof_key_for(proof_ctx);

        // Check if proof already exists
        if self
            .db
            .get_proof(&proof_key)
            .map_err(|e| anyhow::anyhow!("DB error: {}", e))?
            .is_some()
        {
            return Ok(proof_key);
        }

        // Submit task to Prover Service (ignore if already exists)
        match self
            .prover_handle
            .submit_task(ProofTask(proof_ctx), zkvm_backend())
            .await
        {
            Ok(_uuid) => {}
            Err(e) => {
                if !e.to_string().contains("Task already exists") {
                    return Err(anyhow::anyhow!("Failed to submit task: {}", e));
                }
            }
        }

        Ok(proof_key)
    }
}

#[async_trait]
impl StrataProverClientApiServer for ProverClientRpc {
    async fn prove_el_blocks(
        &self,
        el_block_range: (EvmEeBlockCommitment, EvmEeBlockCommitment),
    ) -> RpcResult<Vec<RpcProofKey>> {
        let proof_ctx = ProofContext::EvmEeStf(el_block_range.0, el_block_range.1);

        let proof_key = self
            .submit_task(proof_ctx)
            .await
            .map_err(to_jsonrpsee_error("failed to create task for el block"))?;

        Ok(vec![proof_key])
    }

    async fn prove_checkpoint(&self, ckp_idx: u64) -> RpcResult<Vec<RpcProofKey>> {
        let proof_ctx = ProofContext::Checkpoint(ckp_idx);

        let proof_key = self
            .submit_task(proof_ctx)
            .await
            .map_err(to_jsonrpsee_error(
                "failed to create task for given checkpoint",
            ))?;

        Ok(vec![proof_key])
    }

    async fn prove_latest_checkpoint(&self) -> RpcResult<Vec<RpcProofKey>> {
        #[expect(deprecated, reason = "legacy old code is retained for compatibility")]
        let next_unproven_idx = self
            .checkpoint_operator
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

        let proof_key = self
            .submit_task(proof_ctx)
            .await
            .map_err(to_jsonrpsee_error(
                "failed to create task for next unproven checkpoint",
            ))?;

        Ok(vec![proof_key])
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
            // If proof is not in DB, check Prover Service status
            None => {
                let backend = zkvm_backend();
                // Wrap ProofContext in ProofTask for Prover Service
                let task_id = TaskId::new(ProofTask(*key.context()), backend);

                let status = self
                    .prover_handle
                    .get_status_by_task_id(&task_id)
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
        let summary = self.prover_handle.get_current_status();

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
