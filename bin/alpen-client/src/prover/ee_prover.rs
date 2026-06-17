//! EE prover facade backed by separate chunk and acct paas provers.
//!
//! Chunk proof submission is driven by the chunk lifecycle as soon as chunks are sealed.
//! Acct proof submission is driven by the batch lifecycle only after DA is complete and all
//! chunk proofs for the batch are ready.
//!
//! `check_proof_status(batch_id)` peeks the typed
//! [`EeBatchProofDbManager`] first (proof present → `Ready`); on miss
//! it maps `acct_handle.get_status(BatchTask)` to
//! [`ProofGenerationStatus`].

use std::sync::Arc;

use alpen_ee_common::{
    BatchId, BatchProver, ChunkId, ChunkProver, ChunkStatus, ChunkStorage, Proof,
    ProofGenerationStatus, ProofId,
};
use async_trait::async_trait;
use strata_paas::{ProverError as PaasError, ProverHandle, TaskStatus};
use tracing::{debug, error, info, warn};

use super::{
    spec_acct::AcctSpec, spec_chunk::ChunkSpec, BatchTask, ChunkTask, EeBatchProofDbManager,
};

/// New-paas-backed EE prover facade.
pub(crate) struct PaasEeProver {
    chunk_handle: ProverHandle<ChunkSpec>,
    acct_handle: ProverHandle<AcctSpec>,
    chunk_storage: Arc<dyn ChunkStorage>,
    batch_proofs: Arc<EeBatchProofDbManager>,
}

impl PaasEeProver {
    pub(crate) fn new(
        chunk_handle: ProverHandle<ChunkSpec>,
        acct_handle: ProverHandle<AcctSpec>,
        chunk_storage: Arc<dyn ChunkStorage>,
        batch_proofs: Arc<EeBatchProofDbManager>,
    ) -> Self {
        Self {
            chunk_handle,
            acct_handle,
            chunk_storage,
            batch_proofs,
        }
    }

    async fn observe_existing_chunk_task(&self, task: ChunkTask) -> eyre::Result<bool> {
        let chunk_id = task.0;
        match self.chunk_handle.get_status(&task) {
            Ok(TaskStatus::Completed) => {
                self.chunk_storage
                    .update_chunk_status(chunk_id, ChunkStatus::ProofReady(task.proof_id()))
                    .await?;
                Ok(true)
            }
            Ok(TaskStatus::PermanentFailure { error }) => {
                error!(
                    ?chunk_id,
                    reason = %error,
                    "CRITICAL: chunk proof generation failed permanently; manual intervention required"
                );
                self.chunk_storage
                    .update_chunk_status(chunk_id, ChunkStatus::ProofFailed(error))
                    .await?;
                Ok(true)
            }
            Ok(TaskStatus::Pending)
            | Ok(TaskStatus::Proving { .. })
            | Ok(TaskStatus::TransientFailure { .. }) => {
                self.chunk_storage
                    .update_chunk_status(chunk_id, ChunkStatus::ProofPending(task.to_string()))
                    .await?;
                Ok(true)
            }
            Err(PaasError::TaskNotFound(_)) => Ok(false),
            Err(e) => Err(eyre::eyre!("get_status({chunk_id:?}): {e}")),
        }
    }

    fn observe_existing_batch_task(&self, batch_id: BatchId) -> eyre::Result<bool> {
        if self.batch_proofs.has_proof(batch_id) {
            return Ok(true);
        }

        match self.acct_handle.get_status(&BatchTask(batch_id)) {
            Ok(_) => Ok(true),
            Err(PaasError::TaskNotFound(_)) => Ok(false),
            Err(e) => Err(eyre::eyre!("get_status({batch_id}): {e}")),
        }
    }
}

#[async_trait]
impl ChunkProver for PaasEeProver {
    async fn request_proof_generation(&self, chunk_id: ChunkId) -> eyre::Result<()> {
        let task = ChunkTask(chunk_id);
        let Some((_chunk, status)) = self.chunk_storage.get_chunk_by_id(chunk_id).await? else {
            return Err(eyre::eyre!(
                "cannot submit chunk proof task for missing chunk {chunk_id:?}"
            ));
        };
        match status {
            ChunkStatus::ProofReady(_) => return Ok(()),
            ChunkStatus::ProofFailed(reason) => {
                return Err(eyre::eyre!(
                    "cannot submit chunk proof task for failed chunk {chunk_id:?}: {reason}"
                ));
            }
            ChunkStatus::Sealed | ChunkStatus::ProofPending(_) => {}
        }

        if self.observe_existing_chunk_task(task).await? {
            return Ok(());
        }

        info!(?chunk_id, "submitting chunk proof task");

        self.chunk_handle
            .submit(task)
            .await
            .map_err(|e| eyre::eyre!("submit chunk task {chunk_id:?}: {e}"))?;

        self.chunk_storage
            .update_chunk_status(chunk_id, ChunkStatus::ProofPending(task.to_string()))
            .await?;

        Ok(())
    }

    async fn check_proof_status(&self, chunk_id: ChunkId) -> eyre::Result<ProofGenerationStatus> {
        if let Some((_chunk, status)) = self.chunk_storage.get_chunk_by_id(chunk_id).await? {
            match status {
                ChunkStatus::ProofReady(proof_id) => {
                    return Ok(ProofGenerationStatus::Ready { proof_id });
                }
                ChunkStatus::ProofFailed(reason) => {
                    return Ok(ProofGenerationStatus::Failed { reason });
                }
                ChunkStatus::Sealed | ChunkStatus::ProofPending(_) => {}
            }
        }

        let task = ChunkTask(chunk_id);
        match self.chunk_handle.get_status(&task) {
            Ok(TaskStatus::Completed) => Ok(ProofGenerationStatus::Ready {
                proof_id: task.proof_id(),
            }),
            Ok(TaskStatus::PermanentFailure { error }) => {
                Ok(ProofGenerationStatus::Failed { reason: error })
            }
            Ok(TaskStatus::Pending)
            | Ok(TaskStatus::Proving { .. })
            | Ok(TaskStatus::TransientFailure { .. }) => Ok(ProofGenerationStatus::Pending),
            Err(PaasError::TaskNotFound(_)) => Ok(ProofGenerationStatus::NotStarted),
            Err(e) => {
                warn!(?chunk_id, %e, "chunk_handle.get_status failed");
                Err(eyre::eyre!("get_status({chunk_id:?}): {e}"))
            }
        }
    }
}

#[async_trait]
impl BatchProver for PaasEeProver {
    async fn request_proof_generation(&self, batch_id: BatchId) -> eyre::Result<()> {
        if self.observe_existing_batch_task(batch_id)? {
            return Ok(());
        }

        info!(%batch_id, "submitting acct proof task");

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
            | Ok(TaskStatus::Proving { .. })
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
