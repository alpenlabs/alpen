use alpen_ee_common::{Batch, BatchDaProvider, BatchProver, BatchStatus, BatchStorage};
use eyre::Result;
use tracing::debug;

use crate::batch_lifecycle::{ctx::BatchLifecycleCtx, state::BatchLifecycleState};

/// Try to request proof for the next batch (DaComplete → ProofPending).
pub(crate) async fn try_advance_proof_pending<D, P, S>(
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
    let target_idx = state.proof_pending().idx() + 1;

    // If we're past the latest batch, nothing to do
    if target_idx > latest_batch.idx() {
        return Ok(());
    }

    let Some((batch, status)) = ctx.batch_storage.get_batch_by_idx(target_idx).await? else {
        return Ok(()); // Batch doesn't exist yet
    };

    match status {
        BatchStatus::Sealed | BatchStatus::DaPending => {
            // Not ready, no action
        }
        BatchStatus::DaComplete { da } => {
            // Request proof generation. If this fails, we retry in the next cycle.
            debug!(batch_idx = target_idx, batch_id = ?batch.id(), "Requesting proof");

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
