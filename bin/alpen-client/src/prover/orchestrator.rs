//! Chunk -> account proof orchestration.
//!
//! Manages the two-stage proof pipeline for a single batch:
//! 1. Split batch blocks into chunks, submit chunk proof tasks.
//! 2. Wait for all chunk proofs to complete.
//! 3. Submit account proof task aggregating the chunk proofs.
//! 4. Wait for account proof, store final result.

use std::sync::Arc;

use alpen_ee_common::{BatchId, BatchStorage, EeProofTask, ExecBlockStorage};
use strata_paas::{ProverHandle, TaskStatus, ZkVmBackend};
use tokio::time::{sleep, Duration};
use tracing::{error, info, warn};

use super::proof_store::{BatchProofState, ProofStore};

/// Runs the chunk->acct proof pipeline for a single batch.
///
/// Spawned as a background task by [`super::batch_prover::PaasBatchProver`].
pub(crate) async fn orchestrate_batch_proof(
    batch_id: BatchId,
    chunk_size: usize,
    backend: ZkVmBackend,
    prover_handle: ProverHandle<EeProofTask>,
    proof_store: Arc<ProofStore>,
    batch_storage: Arc<dyn BatchStorage>,
    _block_storage: Arc<dyn ExecBlockStorage>,
) {
    let result = run_pipeline(
        batch_id,
        chunk_size,
        backend,
        &prover_handle,
        &proof_store,
        batch_storage.as_ref(),
    )
    .await;

    match result {
        Ok(()) => {
            info!(%batch_id, "batch proof pipeline completed");
        }
        Err(e) => {
            error!(%batch_id, error = %e, "batch proof pipeline failed");
            proof_store.set_batch_state(
                batch_id,
                BatchProofState::Failed {
                    reason: e.to_string(),
                },
            );
        }
    }
}

async fn run_pipeline(
    batch_id: BatchId,
    chunk_size: usize,
    backend: ZkVmBackend,
    prover_handle: &ProverHandle<EeProofTask>,
    proof_store: &ProofStore,
    batch_storage: &dyn BatchStorage,
) -> eyre::Result<()> {
    // 1. Fetch batch to determine block count.
    let (batch, _status) = batch_storage
        .get_batch_by_id(batch_id)
        .await?
        .ok_or_else(|| eyre::eyre!("batch not found: {batch_id}"))?;

    let block_count = batch.blocks_iter().count();
    let total_chunks = ((block_count + chunk_size - 1) / chunk_size) as u32;

    info!(%batch_id, block_count, total_chunks, chunk_size, "starting chunk proof generation");

    // 2. Submit chunk proof tasks.
    let mut chunk_uuids = Vec::with_capacity(total_chunks as usize);
    for chunk_idx in 0..total_chunks {
        let task = EeProofTask::Chunk {
            batch_id,
            chunk_idx,
        };
        let uuid = prover_handle.submit_task(task, backend.clone()).await?;
        chunk_uuids.push(uuid);
    }

    proof_store.set_batch_state(
        batch_id,
        BatchProofState::ChunksInProgress {
            chunk_uuids: chunk_uuids.clone(),
            total_chunks,
        },
    );

    // 3. Poll until all chunks complete.
    wait_for_tasks(&chunk_uuids, prover_handle, "chunk").await?;
    info!(%batch_id, total_chunks, "all chunk proofs completed");

    // 4. Submit account proof task.
    let acct_task = EeProofTask::Acct { batch_id };
    let acct_uuid = prover_handle
        .submit_task(acct_task, backend)
        .await?;

    proof_store.set_batch_state(
        batch_id,
        BatchProofState::AcctInProgress {
            acct_uuid: acct_uuid.clone(),
        },
    );

    // 5. Wait for account proof.
    wait_for_tasks(&[acct_uuid], prover_handle, "acct").await?;
    info!(%batch_id, "account proof completed");

    // 6. Mark as completed.
    // TODO: extract actual proof bytes from the PaaS receipt and store them.
    // For now, use a deterministic proof_id derived from batch_id.
    let proof_id = ProofStore::proof_id_from_bytes(&batch_id.prev_block().0);
    proof_store.store_proof(proof_id, Vec::new());
    proof_store.set_batch_state(batch_id, BatchProofState::Completed { proof_id });

    Ok(())
}

/// Polls PaaS task statuses until all are completed or one fails permanently.
async fn wait_for_tasks(
    uuids: &[String],
    prover_handle: &ProverHandle<EeProofTask>,
    label: &str,
) -> eyre::Result<()> {
    let poll_interval = Duration::from_secs(5);
    let mut completed = vec![false; uuids.len()];

    loop {
        let mut all_done = true;
        for (i, uuid) in uuids.iter().enumerate() {
            if completed[i] {
                continue;
            }
            match prover_handle.get_status(uuid).await {
                Ok(TaskStatus::Completed) => {
                    completed[i] = true;
                }
                Ok(TaskStatus::PermanentFailure { error }) => {
                    return Err(eyre::eyre!(
                        "{label} task {uuid} permanently failed: {error}"
                    ));
                }
                Ok(_) => {
                    // Pending, Queued, Proving, or TransientFailure — keep waiting.
                    all_done = false;
                }
                Err(e) => {
                    warn!(uuid, error = %e, "{label} task status query failed, retrying");
                    all_done = false;
                }
            }
        }

        if all_done {
            return Ok(());
        }

        sleep(poll_interval).await;
    }
}
