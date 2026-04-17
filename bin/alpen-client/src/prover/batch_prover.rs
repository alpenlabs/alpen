//! [`BatchProver`] impl that drives the chunk + acct paas provers.
//!
//! `request_proof_generation(batch_id)` reads the batch's chunk-id list
//! from `BatchStorage::get_batch_chunks` and submits one `ChunkTask` per
//! chunk + one `BatchTask(batch_id)`. Both submits are idempotent;
//! multi-batch concurrency is paas-native.
//!
//! `check_proof_status(batch_id)` peeks the typed
//! [`EeBatchProofDbManager`] first (proof present → `Ready`); on miss
//! it maps `acct_handle.get_status(BatchTask)` to
//! [`ProofGenerationStatus`].

use std::sync::Arc;

use alpen_ee_common::{BatchId, BatchProver, BatchStorage, Proof, ProofGenerationStatus, ProofId};
use async_trait::async_trait;
use strata_paas::{ProverError as PaasError, ProverHandle, TaskStatus};
use tracing::{debug, info, warn};

use super::{
    spec_acct::AcctSpec, spec_chunk::ChunkSpec, BatchTask, ChunkTask, EeBatchProofDbManager,
};

/// New-paas-backed [`BatchProver`].
pub(crate) struct PaasBatchProver {
    chunk_handle: ProverHandle<ChunkSpec>,
    acct_handle: ProverHandle<AcctSpec>,
    batch_storage: Arc<dyn BatchStorage>,
    batch_proofs: Arc<EeBatchProofDbManager>,
}

impl PaasBatchProver {
    pub(crate) fn new(
        chunk_handle: ProverHandle<ChunkSpec>,
        acct_handle: ProverHandle<AcctSpec>,
        batch_storage: Arc<dyn BatchStorage>,
        batch_proofs: Arc<EeBatchProofDbManager>,
    ) -> Self {
        Self {
            chunk_handle,
            acct_handle,
            batch_storage,
            batch_proofs,
        }
    }
}

#[async_trait]
impl BatchProver for PaasBatchProver {
    async fn request_proof_generation(&self, batch_id: BatchId) -> eyre::Result<()> {
        let chunks = self
            .batch_storage
            .get_batch_chunks(batch_id)
            .await?
            .ok_or_else(|| eyre::eyre!("no chunks set for batch {batch_id}"))?;

        info!(
            %batch_id,
            chunk_count = chunks.len(),
            "submitting chunk + acct proof tasks"
        );

        for chunk_id in chunks {
            let task = ChunkTask(chunk_id);
            self.chunk_handle
                .submit(task)
                .await
                .map_err(|e| eyre::eyre!("submit chunk task {chunk_id:?}: {e}"))?;
        }

        self.acct_handle
            .submit(BatchTask(batch_id))
            .await
            .map_err(|e| eyre::eyre!("submit acct task {batch_id}: {e}"))?;

        Ok(())
    }

    async fn check_proof_status(&self, batch_id: BatchId) -> eyre::Result<ProofGenerationStatus> {
        // Source of truth: the typed batch proof DB (the acct hook writes
        // there). Present ⇒ Ready.
        if self.batch_proofs.has_proof(batch_id) {
            return Ok(ProofGenerationStatus::Ready {
                proof_id: EeBatchProofDbManager::proof_id_for(batch_id),
            });
        }

        // Else map paas's task lifecycle status. `TaskNotFound` ⇒ NotStarted
        // (we never submitted, or we're in a fresh process and haven't yet
        // recovered).
        match self.acct_handle.get_status(&BatchTask(batch_id)) {
            Ok(TaskStatus::Completed) => {
                // Completed but not in the proof DB? Hook hasn't fired yet
                // or the DB lost its entry. Treat as Pending so the
                // lifecycle keeps polling.
                debug!(%batch_id, "acct task Completed but proof not yet in DB; reporting Pending");
                Ok(ProofGenerationStatus::Pending)
            }
            Ok(TaskStatus::PermanentFailure { error }) => {
                Ok(ProofGenerationStatus::Failed { reason: error })
            }
            Ok(TaskStatus::Pending)
            | Ok(TaskStatus::Proving)
            | Ok(TaskStatus::TransientFailure { .. }) => Ok(ProofGenerationStatus::Pending),
            Err(PaasError::TaskNotFound(_)) => Ok(ProofGenerationStatus::NotStarted),
            Err(e) => {
                warn!(%batch_id, %e, "acct_handle.get_status failed");
                Err(eyre::eyre!("get_status({batch_id}): {e}"))
            }
        }
    }

    async fn get_proof(&self, proof_id: ProofId) -> eyre::Result<Option<Proof>> {
        Ok(self.batch_proofs.get_proof_by_id(proof_id))
    }
}
