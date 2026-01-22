use alpen_ee_common::{Batch, BatchDaProvider, BatchProver, BatchStatus, BatchStorage};
use eyre::Result;
use tracing::debug;

use crate::batch_lifecycle::{ctx::BatchLifecycleCtx, state::BatchLifecycleState};

/// Try to post DA for the next batch (Sealed → DaPending).
pub(crate) async fn try_advance_da_pending<D, P, S>(
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
    let target_idx = state.da_pending().idx() + 1;

    // If we're past the latest batch, nothing to do
    if target_idx > latest_batch.idx() {
        return Ok(());
    }

    let Some((batch, status)) = ctx.batch_storage.get_batch_by_idx(target_idx).await? else {
        return Ok(()); // Batch doesn't exist yet
    };

    match status {
        BatchStatus::Sealed => {
            // Start DA posting. If this fails, we retry in the next cycle.
            debug!(batch_idx = target_idx, batch_id = ?batch.id(), "Posting DA");

            ctx.da_provider.post_batch_da(batch.id()).await?;

            ctx.batch_storage
                .update_batch_status(batch.id(), BatchStatus::DaPending)
                .await?;

            state.advance_da_pending(target_idx, batch.id());
        }
        BatchStatus::DaPending
        | BatchStatus::DaComplete { .. }
        | BatchStatus::ProofPending { .. }
        | BatchStatus::ProofReady { .. }
        | BatchStatus::Genesis => {
            // Already past this stage, advance frontier
            state.advance_da_pending(target_idx, batch.id());
        }
    }

    Ok(())
}
