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

use super::{ol_stf::OLStfOperator, ProofInputFetcher};
use crate::{
    checkpoint_runner::{errors::CheckpointResult, submit::submit_checkpoint_proof},
    errors::ProvingTaskError,
};

/// Operator for checkpoint proof generation.
///
/// Provides access to OL client and checkpoint submission functionality.
#[derive(Debug, Clone)]
pub(crate) struct CheckpointOperator {
    ol_client: HttpClient,
    ol_stf_operator: Arc<OLStfOperator>,
}

impl CheckpointOperator {
    /// Creates a new checkpoint operator.
    pub(crate) fn new(ol_client: HttpClient, ol_stf_operator: Arc<OLStfOperator>) -> Self {
        Self {
            ol_client,
            ol_stf_operator,
        }
    }

    /// Fetches checkpoint information from the OL client.
    pub(crate) async fn fetch_ckp_info(
        &self,
        ckp_idx: u64,
    ) -> Result<RpcCheckpointInfo, ProvingTaskError> {
        self.ol_client
            .get_checkpoint_info(ckp_idx)
            .await
            .inspect_err(|_| error!(%ckp_idx, "Failed to fetch CheckpointInfo"))
            .map_err(|e| ProvingTaskError::RpcError(e.to_string()))?
            .ok_or(ProvingTaskError::WitnessNotFound)
    }

    /// Returns a reference to the internal OL (Orchestration Layer) [`HttpClient`].
    pub(crate) fn ol_client(&self) -> &HttpClient {
        &self.ol_client
    }

    /// Returns a reference to the OLStf operator.
    pub(crate) fn ol_stf_operator(&self) -> &Arc<OLStfOperator> {
        &self.ol_stf_operator
    }

    /// Creates and stores the OL Stf proof dependencies for a checkpoint.
    ///
    /// This fetches the checkpoint info from the OL client and creates a OL Stf proof context
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

        info!(%ckp_idx, "Creating OLStf dependency for checkpoint");

        // Create OLStf proof context from the checkpoint's L2 range
        let ol_stf_ctx = ProofContext::OLStf(ckp_info.l2_range.0, ckp_info.l2_range.1);

        // Store Checkpoint dependencies (OLStf)
        db.put_proof_deps(checkpoint_ctx, vec![ol_stf_ctx])
            .map_err(ProvingTaskError::DatabaseError)?;

        Ok(vec![ol_stf_ctx])
    }

    /// Submits a checkpoint proof to the OL client.
    pub(crate) async fn submit_checkpoint_proof(
        &self,
        checkpoint_index: u64,
        proof_key: &ProofKey,
        proof_db: &ProofDBSled,
    ) -> CheckpointResult<()> {
        submit_checkpoint_proof(checkpoint_index, self.ol_client(), proof_key, proof_db).await
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

        assert!(!deps.is_empty(), "checkpoint must have some OL STF proofs");

        let ol_stf_key = ProofKey::new(deps[0], *task_id.host());
        let ol_stf_vk = get_verification_key(&ol_stf_key);

        let mut ol_stf_proofs = Vec::with_capacity(deps.len());
        for dep in deps {
            // Validate that all dependencies are OLStf proofs
            match dep {
                strata_primitives::proof::ProofContext::OLStf(..) => {}
                _ => panic!(
                    "Checkpoint dependencies must be OLStf proofs, got: {:?}",
                    dep
                ),
            };
            let ol_stf_key = ProofKey::new(dep, *task_id.host());
            let proof = db
                .get_proof(&ol_stf_key)
                .map_err(ProvingTaskError::DatabaseError)?
                .ok_or(ProvingTaskError::ProofNotFound(ol_stf_key))?;
            ol_stf_proofs.push(proof);
        }

        Ok(CheckpointProverInput {
            ol_stf_proofs,
            ol_stf_vk,
        })
    }
}
