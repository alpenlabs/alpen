//! [`BatchProver`] implementation backed by PaaS.

use std::sync::Arc;

use alpen_ee_common::{
    BatchId, BatchProver, BatchStorage, EeProofTask, ExecBlockStorage, Proof,
    ProofGenerationStatus, ProofId,
};
use async_trait::async_trait;
use strata_paas::{ProverHandle, ZkVmBackend};
use tracing::info;

use super::{
    orchestrator::orchestrate_batch_proof,
    proof_store::{BatchProofState, ProofStore},
};

/// [`BatchProver`] implementation that uses PaaS for proof generation.
///
/// Orchestrates a two-stage pipeline (chunk proofs -> account proof) internally.
/// The batch lifecycle sees a single proof per batch.
pub(crate) struct PaasBatchProver {
    prover_handle: ProverHandle<EeProofTask>,
    proof_store: Arc<ProofStore>,
    batch_storage: Arc<dyn BatchStorage>,
    block_storage: Arc<dyn ExecBlockStorage>,
    chunk_size: usize,
    backend: ZkVmBackend,
}

impl PaasBatchProver {
    pub(crate) fn new(
        prover_handle: ProverHandle<EeProofTask>,
        proof_store: Arc<ProofStore>,
        batch_storage: Arc<dyn BatchStorage>,
        block_storage: Arc<dyn ExecBlockStorage>,
        chunk_size: usize,
        backend: ZkVmBackend,
    ) -> Self {
        Self {
            prover_handle,
            proof_store,
            batch_storage,
            block_storage,
            chunk_size,
            backend,
        }
    }
}

#[async_trait]
impl BatchProver for PaasBatchProver {
    async fn request_proof_generation(&self, batch_id: BatchId) -> eyre::Result<()> {
        // Idempotent: skip if already in progress or completed.
        if let Some(state) = self.proof_store.get_batch_state(&batch_id) {
            match state {
                BatchProofState::Completed { .. }
                | BatchProofState::ChunksInProgress { .. }
                | BatchProofState::AcctInProgress { .. } => {
                    info!(%batch_id, ?state, "proof generation already in progress or completed");
                    return Ok(());
                }
                BatchProofState::Failed { .. } => {
                    // Allow retry on previous failure.
                    info!(%batch_id, "retrying previously failed proof generation");
                }
            }
        }

        info!(%batch_id, "requesting proof generation");

        // Spawn background orchestrator task.
        let batch_id_owned = batch_id;
        let chunk_size = self.chunk_size;
        let backend = self.backend.clone();
        let prover_handle = self.prover_handle.clone();
        let proof_store = self.proof_store.clone();
        let batch_storage = self.batch_storage.clone();
        let block_storage = self.block_storage.clone();

        tokio::spawn(async move {
            orchestrate_batch_proof(
                batch_id_owned,
                chunk_size,
                backend,
                prover_handle,
                proof_store,
                batch_storage,
                block_storage,
            )
            .await;
        });

        Ok(())
    }

    async fn check_proof_status(
        &self,
        batch_id: BatchId,
    ) -> eyre::Result<ProofGenerationStatus> {
        match self.proof_store.get_batch_state(&batch_id) {
            Some(BatchProofState::Completed { proof_id }) => {
                Ok(ProofGenerationStatus::Ready { proof_id })
            }
            Some(BatchProofState::Failed { reason }) => {
                Ok(ProofGenerationStatus::Failed { reason })
            }
            Some(BatchProofState::ChunksInProgress { .. })
            | Some(BatchProofState::AcctInProgress { .. }) => {
                Ok(ProofGenerationStatus::Pending)
            }
            None => Ok(ProofGenerationStatus::NotStarted),
        }
    }

    async fn get_proof(&self, proof_id: ProofId) -> eyre::Result<Option<Proof>> {
        Ok(self.proof_store.get_proof(&proof_id))
    }
}
