use jsonrpsee::http_client::HttpClient;
use strata_checkpoint_types::BatchInfo;
use strata_db_store_sled::prover::ProofDBSled;
use strata_primitives::proof::{ProofContext, ProofKey};
use strata_rpc_api::StrataApiClient;
#[expect(deprecated, reason = "legacy old code is retained for compatibility")]
use strata_rpc_types::RpcCheckpointInfo;
use tracing::error;

use super::ProofInputFetcher;
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
}

impl CheckpointOperator {
    /// Creates a new checkpoint operator.
    pub(crate) fn new(cl_client: HttpClient) -> Self {
        Self { cl_client }
    }

    /// Fetches checkpoint information from the CL client.
    #[expect(deprecated, reason = "legacy old code is retained for compatibility")]
    pub(crate) async fn fetch_ckp_info(
        &self,
        ckp_idx: u64,
    ) -> Result<RpcCheckpointInfo, ProvingTaskError> {
        #[expect(deprecated, reason = "legacy old code is retained for compatibility")]
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
    type Input = BatchInfo;

    #[expect(deprecated, reason = "legacy old code is retained for compatibility")]
    async fn fetch_input(
        &self,
        task_id: &ProofKey,
        _db: &ProofDBSled,
    ) -> Result<Self::Input, ProvingTaskError> {
        let ckp_idx = match task_id.context() {
            ProofContext::Checkpoint(idx) => *idx,
            _ => return Err(ProvingTaskError::InvalidInput("Checkpoint".to_string())),
        };

        let ckp_info = self.fetch_ckp_info(ckp_idx).await?;

        Ok(BatchInfo::new(
            ckp_info.idx as u32,
            ckp_info.l1_range,
            ckp_info.l2_range,
        ))
    }
}
