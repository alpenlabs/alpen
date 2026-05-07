use alpen_ee_common::{Batch, BatchDaProvider, BatchProver, BatchStatus, BatchStorage};
use eyre::Result;
use tracing::{debug, warn};

use crate::batch_lifecycle::{ctx::BatchLifecycleCtx, state::BatchLifecycleState};

/// Posts DA for as many sealed batches as possible (Sealed → DaPending).
///
/// Each batch is submitted as its own chunked-envelope tx set, so posting the
/// next batch does not depend on earlier batches reaching DA readiness. The
/// frontier advances while sealed batches have their state diffs available and
/// `post_batch_da` succeeds.
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
    loop {
        let target_idx = state.da_pending().idx() + 1;
        if target_idx > latest_batch.idx() {
            return Ok(());
        }

        let Some((batch, status)) = ctx.batch_storage.get_batch_by_idx(target_idx).await? else {
            return Ok(());
        };

        match status {
            BatchStatus::Sealed => {
                let batch_id = batch.id();
                if !ctx.blob_provider.are_state_diffs_ready(batch_id).await {
                    warn!(
                        batch_idx = target_idx,
                        batch_id = ?batch_id,
                        "State diffs not ready, waiting"
                    );
                    return Ok(());
                }

                debug!(batch_idx = target_idx, batch_id = ?batch_id, "Posting DA");

                let envelope_idx = match ctx.da_provider.post_batch_da(batch_id).await {
                    Ok(envelope_idx) => envelope_idx,
                    Err(e) => {
                        warn!(
                            batch_idx = target_idx,
                            batch_id = ?batch_id,
                            error = %e,
                            "failed to post batch DA; will retry next lifecycle tick"
                        );
                        return Ok(());
                    }
                };

                ctx.batch_storage
                    .update_batch_status(batch_id, BatchStatus::DaPending { envelope_idx })
                    .await?;

                state.advance_da_pending(target_idx, batch_id);
            }
            BatchStatus::DaPending { .. }
            | BatchStatus::DaComplete { .. }
            | BatchStatus::ProofPending { .. }
            | BatchStatus::ProofReady { .. }
            | BatchStatus::Genesis => {
                state.advance_da_pending(target_idx, batch.id());
            }
        }
    }
}
