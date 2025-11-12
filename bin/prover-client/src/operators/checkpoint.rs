use std::sync::Arc;

use jsonrpsee::http_client::HttpClient;
use strata_db_store_sled::prover::ProofDBSled;
use strata_db_types::traits::ProofDatabase;
use strata_primitives::proof::ProofKey;
use strata_proofimpl_checkpoint::program::CheckpointProverInput;
use strata_rpc_api::StrataApiClient;
use strata_rpc_types::RpcCheckpointInfo;
use strata_zkvm_hosts::get_verification_key;
use tracing::error;

use super::cl_stf::ClStfOperator;
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
    enable_checkpoint_runner: bool,
}

impl CheckpointOperator {
    /// Creates a new checkpoint operator.
    pub(crate) fn new(
        cl_client: HttpClient,
        cl_stf_operator: Arc<ClStfOperator>,
        enable_checkpoint_runner: bool,
    ) -> Self {
        Self {
            cl_client,
            cl_stf_operator,
            enable_checkpoint_runner,
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

    /// Fetches the input required for checkpoint proof generation.
    ///
    /// This is used by the PaaS integration layer to fetch inputs for proving tasks.
    pub(crate) async fn fetch_input(
        &self,
        task_id: &ProofKey,
        db: &ProofDBSled,
    ) -> Result<CheckpointProverInput, ProvingTaskError> {
        let deps = db
            .get_proof_deps(*task_id.context())
            .map_err(ProvingTaskError::DatabaseError)?
            .ok_or(ProvingTaskError::DependencyNotFound(*task_id))?;

        assert!(!deps.is_empty(), "checkpoint must have some CL STF proofs");

        let cl_stf_key = ProofKey::new(deps[0], *task_id.host());
        let cl_stf_vk = get_verification_key(&cl_stf_key);

        let mut cl_stf_proofs = Vec::with_capacity(deps.len());
        for dep in deps {
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

    /// Returns a reference to the internal CL (Consensus Layer) [`HttpClient`].
    pub(crate) fn cl_client(&self) -> &HttpClient {
        &self.cl_client
    }

    /// Submits a checkpoint proof to the CL client.
    pub(crate) async fn submit_checkpoint_proof(
        &self,
        checkpoint_index: u64,
        proof_key: &ProofKey,
        proof_db: &ProofDBSled,
    ) -> CheckpointResult<()> {
        if !self.enable_checkpoint_runner {
            return Ok(());
        }
        submit_checkpoint_proof(checkpoint_index, self.cl_client(), proof_key, proof_db).await
    }
}

