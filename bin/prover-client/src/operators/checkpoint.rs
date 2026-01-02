use std::sync::Arc;

use jsonrpsee::http_client::HttpClient;
use strata_db_store_sled::prover::ProofDBSled;
use strata_db_types::traits::ProofDatabase;
use strata_primitives::proof::{ProofContext, ProofKey};
use strata_proofimpl_checkpoint::program::CheckpointProverInput;
use strata_rpc_api::StrataApiClient;
use strata_rpc_types::RpcCheckpointInfo;
use strata_zkvm_hosts::get_verification_key;
use tracing::{error, info};

use super::{cl_stf::ClStfOperator, ProofInputFetcher};
use crate::{
    checkpoint_runner::{errors::CheckpointResult, submit::submit_checkpoint_proof},
    errors::ProvingTaskError,
};

/// Operator for checkpoint proof generation.
///
/// Provides access to CL client and checkpoint submission functionality.
#[derive(Debug, Clone)]
pub(crate) struct CheckpointOperator {
    cl_client: HttpClient,
    cl_stf_operator: Arc<ClStfOperator>,
}

impl CheckpointOperator {
    /// Creates a new checkpoint operator.
    pub(crate) fn new(cl_client: HttpClient, cl_stf_operator: Arc<ClStfOperator>) -> Self {
        Self {
            cl_client,
            cl_stf_operator,
        }
    }

    /// Fetches checkpoint information from the CL client.
    pub(crate) async fn fetch_ckp_info(
        &self,
        ckp_idx: u64,
    ) -> Result<RpcCheckpointInfo, ProvingTaskError> {
        self.cl_client
            .get_checkpoint_info(ckp_idx)
            .await
            .inspect_err(|_| error!(%ckp_idx, "Failed to fetch CheckpointInfo"))
            .map_err(|e| ProvingTaskError::RpcError(e.to_string()))?
            .ok_or(ProvingTaskError::WitnessNotFound)
    }

    /// Returns a reference to the internal CL (Consensus Layer) [`HttpClient`].
    pub(crate) fn cl_client(&self) -> &HttpClient {
        &self.cl_client
    }

    /// Returns a reference to the ClStf operator.
    pub(crate) fn cl_stf_operator(&self) -> &Arc<ClStfOperator> {
        &self.cl_stf_operator
    }

    /// Creates and stores the ClStf proof dependencies for a checkpoint.
    ///
    /// This fetches the checkpoint info from the CL client and creates a ClStf proof context
    /// for the L2 block range covered by the checkpoint.
    pub(crate) async fn create_checkpoint_deps(
        &self,
        ckp_idx: u64,
        db: &ProofDBSled,
    ) -> Result<Vec<ProofContext>, ProvingTaskError> {
        // Check if dependencies already exist
        let checkpoint_ctx = ProofContext::Checkpoint(ckp_idx);
        if let Some(existing_deps) = db
            .get_proof_deps(checkpoint_ctx)
            .map_err(ProvingTaskError::DatabaseError)?
        {
            info!(%ckp_idx, "Checkpoint dependencies already exist, skipping creation");
            return Ok(existing_deps);
        }

        // Fetch checkpoint info to get L2 range
        let ckp_info = self.fetch_ckp_info(ckp_idx).await?;

        info!(%ckp_idx, "Creating ClStf dependency for checkpoint");

        // Create ClStf proof context from the checkpoint's L2 range
        let cl_stf_ctx = ProofContext::ClStf(ckp_info.l2_range.0, ckp_info.l2_range.1);

        // Store Checkpoint dependencies (ClStf)
        db.put_proof_deps(checkpoint_ctx, vec![cl_stf_ctx])
            .map_err(ProvingTaskError::DatabaseError)?;

        Ok(vec![cl_stf_ctx])
    }

    /// Submits a checkpoint proof to the CL client.
    pub(crate) async fn submit_checkpoint_proof(
        &self,
        checkpoint_index: u64,
        proof_key: &ProofKey,
        proof_db: &ProofDBSled,
    ) -> CheckpointResult<()> {
        submit_checkpoint_proof(checkpoint_index, self.cl_client(), proof_key, proof_db).await
    }
}

impl ProofInputFetcher for CheckpointOperator {
    type Input = CheckpointProverInput;

    async fn fetch_input(
        &self,
        task_id: &ProofKey,
        db: &ProofDBSled,
    ) -> Result<Self::Input, ProvingTaskError> {
        let deps = db
            .get_proof_deps(*task_id.context())
            .map_err(ProvingTaskError::DatabaseError)?
            .ok_or(ProvingTaskError::DependencyNotFound(*task_id))?;

        assert!(!deps.is_empty(), "checkpoint must have some CL STF proofs");

        let cl_stf_key = ProofKey::new(deps[0], *task_id.host());
        let cl_stf_vk = get_verification_key(&cl_stf_key);

        let mut cl_stf_proofs = Vec::with_capacity(deps.len());
        for dep in deps {
            // Validate that all dependencies are ClStf proofs
            match dep {
                ProofContext::ClStf(..) => {}
                _ => panic!(
                    "Checkpoint dependencies must be ClStf proofs, got: {:?}",
                    dep
                ),
            };
            let cl_stf_key = ProofKey::new(dep, *task_id.host());
            let proof = db
                .get_proof(&cl_stf_key)
                .map_err(ProvingTaskError::DatabaseError)?
                .ok_or(ProvingTaskError::ProofNotFound(cl_stf_key))?;
            cl_stf_proofs.push(proof);
        }

        Ok(CheckpointProverInput {
            cl_stf_proofs,
            cl_stf_vk,
        })
    }
}
