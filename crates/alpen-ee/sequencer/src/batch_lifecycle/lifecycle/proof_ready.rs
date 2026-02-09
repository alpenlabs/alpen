use alpen_ee_common::{
    Batch, BatchDaProvider, BatchProver, BatchStatus, BatchStorage, ProofGenerationStatus,
};
use eyre::Result;
use tracing::{debug, error, warn};

use crate::batch_lifecycle::{ctx::BatchLifecycleCtx, state::BatchLifecycleState};

/// Try to complete proof for the next batch (ProofPending → ProofReady).
pub(crate) async fn try_advance_proof_ready<D, P, S>(
    state: &mut BatchLifecycleState,
    latest_batch: &Batch,
    ctx: &BatchLifecycleCtx<D, P, S>,
) -> Result<()>
where
    D: BatchDaProvider,
    P: BatchProver,
    S: BatchStorage,
{
    // Next batch to process is current frontier + 1
    let target_idx = state.proof_ready().idx() + 1;

    // If we're past the latest batch, nothing to do
    if target_idx > latest_batch.idx() {
        return Ok(());
    }

    let Some((batch, status)) = ctx.batch_storage.get_batch_by_idx(target_idx).await? else {
        return Ok(()); // Batch doesn't exist yet
    };

    match status {
        BatchStatus::Sealed | BatchStatus::DaPending { .. } | BatchStatus::DaComplete { .. } => {
            // Not ready, no action
        }
        BatchStatus::ProofPending { da } => {
            // Check proof status
            match ctx.prover.check_proof_status(batch.id()).await? {
                ProofGenerationStatus::Ready { proof_id } => {
                    debug!(batch_idx = target_idx, batch_id = ?batch.id(), "Proof ready");

                    ctx.batch_storage
                        .update_batch_status(
                            batch.id(),
                            BatchStatus::ProofReady {
                                da,
                                proof: proof_id,
                            },
                        )
                        .await?;

                    // Notify watchers
                    let _ = ctx.proof_ready_tx.send(Some(batch.id()));

                    state.advance_proof_ready(target_idx, batch.id());
                }

                ProofGenerationStatus::Failed { reason } => {
                    // CRITICAL: Manual intervention required
                    error!(
                        batch_idx = target_idx,
                        batch_id = ?batch.id(),
                        reason = %reason,
                        "CRITICAL: Proof generation failed - manual intervention required. \
                         Batch is stuck in ProofPending state."
                    );
                    // Stay at frontier - manual intervention required
                }

                ProofGenerationStatus::Pending => {
                    // Still waiting, no action
                }

                ProofGenerationStatus::NotStarted => {
                    // We've marked the batch as proof pending, but prover says proof generation has
                    // not started. Try to re-request proof generation and hope for the best.
                    warn!(
                        batch_idx = target_idx,
                        batch_id = ?batch.id(),
                        "Expected proof generation to have been started. Retrying proof generation"
                    );

                    ctx.prover.request_proof_generation(batch.id()).await?;
                }
            }
        }
        BatchStatus::ProofReady { .. } | BatchStatus::Genesis => {
            // Already complete, advance frontier
            state.advance_proof_ready(target_idx, batch.id());
        }
    }

    Ok(())
}
