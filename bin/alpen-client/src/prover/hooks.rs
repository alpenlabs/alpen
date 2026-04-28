//! Receipt hooks for the chunk + acct provers.
//!
//! The chunk hook flips `ChunkStatus::ProofReady` in EE storage so the
//! batch lifecycle and the acct prover can observe completion. The acct
//! hook persists the outer proof into [`EeBatchProofDbManager`] and
//! flips `BatchStatus::ProofReady` so the lifecycle task can post the
//! `EEUpdate` to OL.

use std::sync::Arc;

use alpen_ee_common::{BatchStatus, BatchStorage, ChunkStatus};
use async_trait::async_trait;
use strata_paas::{ProverError, ProverResult, ReceiptHook};
use tracing::{info, warn};
use zkaleido::ProofReceiptWithMetadata;

use super::{
    spec_acct::AcctSpec, spec_chunk::ChunkSpec, BatchTask, ChunkTask, EeBatchProofDbManager,
};

/// Hook fired after a chunk proof is stored in paas's `ReceiptStore`.
///
/// `ProofId` for a chunk is its `last_block` hash (see
/// [`EeBatchProofDbManager::proof_id_for`] for the analogous batch
/// convention). That keeps `ChunkStatus::ProofReady(ProofId)` aligned
/// with the proof's actual identity in storage.
pub(crate) struct ChunkReceiptHook {
    batch_storage: Arc<dyn BatchStorage>,
}

impl ChunkReceiptHook {
    pub(crate) fn new(batch_storage: Arc<dyn BatchStorage>) -> Self {
        Self { batch_storage }
    }
}

#[async_trait]
impl ReceiptHook<ChunkSpec> for ChunkReceiptHook {
    async fn on_receipt(
        &self,
        task: &ChunkTask,
        _receipt: &ProofReceiptWithMetadata,
    ) -> ProverResult<()> {
        let chunk_id = task.0;
        let proof_id = chunk_id.last_block();
        info!(?chunk_id, %proof_id, "marking chunk as proof-ready");
        self.batch_storage
            .update_chunk_status(chunk_id, ChunkStatus::ProofReady(proof_id))
            .await
            .map_err(|e| ProverError::Storage(format!("update_chunk_status: {e}")))
    }
}

/// Hook fired after an acct (outer/update) proof is stored.
///
/// 1. Persists the receipt to [`EeBatchProofDbManager`] keyed by `BatchId`.
/// 2. Reads the current `BatchStatus`; expects `ProofPending { da }` so the existing DA refs carry
///    through to `ProofReady`.
/// 3. Flips status to `BatchStatus::ProofReady { da, proof: <proof_id> }`.
pub(crate) struct AcctReceiptHook {
    batch_storage: Arc<dyn BatchStorage>,
    batch_proofs: Arc<EeBatchProofDbManager>,
}

impl AcctReceiptHook {
    pub(crate) fn new(
        batch_storage: Arc<dyn BatchStorage>,
        batch_proofs: Arc<EeBatchProofDbManager>,
    ) -> Self {
        Self {
            batch_storage,
            batch_proofs,
        }
    }
}

#[async_trait]
impl ReceiptHook<AcctSpec> for AcctReceiptHook {
    async fn on_receipt(
        &self,
        task: &BatchTask,
        receipt: &ProofReceiptWithMetadata,
    ) -> ProverResult<()> {
        let batch_id = task.0;
        let proof_id = EeBatchProofDbManager::proof_id_for(batch_id);
        info!(%batch_id, %proof_id, "persisting batch acct proof");

        self.batch_proofs
            .put_proof(batch_id, receipt.clone())
            .map_err(|e| ProverError::Storage(format!("put_proof({batch_id}): {e}")))?;

        // Read current status to extract the DA refs the lifecycle attached
        // when the batch transitioned DaComplete → ProofPending.
        let (_batch, status) = self
            .batch_storage
            .get_batch_by_id(batch_id)
            .await
            .map_err(|e| ProverError::Storage(format!("get_batch_by_id: {e}")))?
            .ok_or_else(|| ProverError::Storage(format!("batch not found: {batch_id}")))?;

        let da = match status {
            BatchStatus::ProofPending { da } => da,
            BatchStatus::ProofReady { .. } => {
                warn!(%batch_id, "acct hook fired for batch already ProofReady; idempotent skip");
                return Ok(());
            }
            other => {
                return Err(ProverError::Storage(format!(
                    "acct hook expected ProofPending, found {other:?} for {batch_id}"
                )))
            }
        };

        self.batch_storage
            .update_batch_status(
                batch_id,
                BatchStatus::ProofReady {
                    da,
                    proof: proof_id,
                },
            )
            .await
            .map_err(|e| ProverError::Storage(format!("update_batch_status: {e}")))
    }
}
