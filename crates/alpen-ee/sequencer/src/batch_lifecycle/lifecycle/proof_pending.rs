use alpen_ee_common::{
    Batch, BatchDaProvider, BatchProver, BatchStatus, BatchStorage, ChunkStorage,
};
use eyre::Result;
use tracing::{debug, error, warn};

use crate::batch_lifecycle::{
    acct_proof_gate::{check_acct_proof_gate, AcctProofGateDecision},
    ctx::BatchLifecycleCtx,
    state::BatchLifecycleState,
};

/// Try to request acct proof for the next batch (DaComplete → ProofPending).
///
/// Chunk proofs are tracked independently in [`ChunkStatus`]. A DA-complete batch only enters
/// `ProofPending` once every chunk in the batch is `ProofReady`; at that point the acct proof task
/// has all chunk receipts it needs as input.
pub(crate) async fn try_advance_proof_pending<D, P, S>(
    state: &mut BatchLifecycleState,
    latest_batch: &Batch,
    ctx: &BatchLifecycleCtx<D, P, S>,
) -> Result<()>
where
    D: BatchDaProvider,
    P: BatchProver,
    S: BatchStorage + ChunkStorage,
{
    // Next batch to process is current frontier + 1
    let target_idx = state.proof_pending().idx() + 1;

    // If we're past the latest batch, nothing to do
    if target_idx > latest_batch.idx() {
        return Ok(());
    }

    let Some((batch, status)) = ctx.batch_storage.get_batch_by_idx(target_idx).await? else {
        return Ok(()); // Batch doesn't exist yet
    };

    match status {
        BatchStatus::Sealed | BatchStatus::DaPending { .. } => {
            // Not ready, no action
        }
        BatchStatus::DaComplete { da } => {
            if !acct_proof_inputs_ready(&batch, target_idx, ctx.batch_storage.as_ref()).await? {
                return Ok(());
            }

            // Request acct proof generation. If this fails, we retry in the next cycle.
            debug!(batch_idx = target_idx, batch_id = ?batch.id(), "requesting acct proof");

            ctx.prover.request_proof_generation(batch.id()).await?;

            ctx.batch_storage
                .update_batch_status(batch.id(), BatchStatus::ProofPending { da })
                .await?;

            state.advance_proof_pending(target_idx, batch.id());
        }
        BatchStatus::ProofPending { .. }
        | BatchStatus::ProofReady { .. }
        | BatchStatus::Genesis => {
            // Already past this stage, advance frontier
            state.advance_proof_pending(target_idx, batch.id());
        }
    }

    Ok(())
}

/// Evaluate the acct proof gate for `batch`, log the outcome, and report whether the acct proof may
/// be requested now (i.e. all of the batch's chunks are proven).
async fn acct_proof_inputs_ready(
    batch: &Batch,
    target_idx: u64,
    chunk_storage: &impl ChunkStorage,
) -> Result<bool> {
    match check_acct_proof_gate(batch, chunk_storage).await? {
        AcctProofGateDecision::Ready => Ok(true),
        AcctProofGateDecision::WaitingForChunkLinks => {
            debug!(
                batch_idx = target_idx,
                batch_id = ?batch.id(),
                "waiting for batch chunk links before requesting acct proof"
            );
            Ok(false)
        }
        AcctProofGateDecision::WaitingForChunkProof { chunk_id, status } => {
            debug!(
                batch_idx = target_idx,
                batch_id = ?batch.id(),
                ?chunk_id,
                ?status,
                "waiting for chunk proof before requesting acct proof"
            );
            Ok(false)
        }
        AcctProofGateDecision::WaitingForMissingChunk { chunk_id } => {
            warn!(
                batch_idx = target_idx,
                batch_id = ?batch.id(),
                ?chunk_id,
                "batch references a chunk not in storage; waiting (transient during reorg, or \
                 storage corruption if it persists)"
            );
            Ok(false)
        }
        AcctProofGateDecision::BlockedByChunkFailure { chunk_id, reason } => {
            // TODO(str-TBD): this fires on every poll while the batch stays blocked. Make it
            // edge-triggered (log once on transition into blocked, plus a metric) instead of
            // re-logging at error level each tick.
            error!(
                batch_idx = target_idx,
                batch_id = ?batch.id(),
                ?chunk_id,
                %reason,
                "CRITICAL: chunk proof failed; acct proof request is blocked and needs manual \
                 intervention"
            );
            Ok(false)
        }
    }
}
