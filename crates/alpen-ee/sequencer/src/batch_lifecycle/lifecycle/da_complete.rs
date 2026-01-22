use alpen_ee_common::{Batch, BatchDaProvider, BatchProver, BatchStatus, BatchStorage, DaStatus};
use eyre::Result;
use tracing::{debug, error, warn};

use crate::batch_lifecycle::{ctx::BatchLifecycleCtx, state::BatchLifecycleState};

/// Try to confirm DA for the next batch (DaPending → DaComplete).
pub(crate) async fn try_advance_da_complete<D, P, S>(
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
    let target_idx = state.da_complete().idx() + 1;

    // If we're past the latest batch, nothing to do
    if target_idx > latest_batch.idx() {
        return Ok(());
    }

    let Some((batch, status)) = ctx.batch_storage.get_batch_by_idx(target_idx).await? else {
        return Ok(()); // Batch doesn't exist yet
    };

    match status {
        BatchStatus::Sealed => {
            // Not ready, no action
        }
        BatchStatus::DaPending => {
            // Check if DA is confirmed
            match ctx.da_provider.check_da_status(batch.id()).await? {
                DaStatus::Pending => {
                    // Not ready, no action
                }
                DaStatus::Ready(da_refs) => {
                    debug!(batch_idx = target_idx, batch_id = ?batch.id(), "DA confirmed");

                    ctx.batch_storage
                        .update_batch_status(batch.id(), BatchStatus::DaComplete { da: da_refs })
                        .await?;

                    state.advance_da_complete(target_idx, batch.id());
                }
                DaStatus::NotRequested => {
                    // We've marked the batch as da pending, but da provider says da has not been
                    // requested. Try to re-request and hope for the best.
                    warn!(
                        batch_idx = target_idx,
                        batch_id = ?batch.id(),
                        "Expected da operation to have been started. Retrying"
                    );

                    ctx.da_provider.post_batch_da(batch.id()).await?;
                }
                DaStatus::Failed { reason } => {
                    // CRITICAL: Manual intervention required
                    error!(
                        batch_idx = target_idx,
                        batch_id = ?batch.id(),
                        reason = %reason,
                        "CRITICAL: DA posting failed - manual intervention required. \
                         Batch is stuck in DaPending state."
                    );
                    // Stay at frontier - manual intervention required
                }
            };
        }
        BatchStatus::DaComplete { .. }
        | BatchStatus::ProofPending { .. }
        | BatchStatus::ProofReady { .. }
        | BatchStatus::Genesis => {
            // Already past this stage, advance frontier
            state.advance_da_complete(target_idx, batch.id());
        }
    }

    Ok(())
}
