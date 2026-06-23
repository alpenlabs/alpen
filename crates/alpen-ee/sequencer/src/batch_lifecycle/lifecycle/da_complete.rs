use alpen_ee_common::{Batch, BatchDaProvider, BatchProver, BatchStatus, BatchStorage, DaStatus};
use eyre::Result;
use tracing::{debug, error, warn};

use crate::batch_lifecycle::{ctx::BatchLifecycleCtx, state::BatchLifecycleState};

/// Marks DA complete for as many batches as possible (DaPending → DaComplete).
///
/// The DA provider returns [`DaStatus::Ready`] only after the batch's DA txs
/// reach the provider's readiness threshold. The frontier advances through
/// consecutive ready batches and stops at the first sealed or still-pending
/// batch.
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
    loop {
        let target_idx = state.da_complete().idx() + 1;
        if target_idx > latest_batch.idx() {
            return Ok(());
        }

        let Some((batch, status)) = ctx.batch_storage.get_batch_by_idx(target_idx).await? else {
            return Ok(());
        };

        match status {
            BatchStatus::Sealed => {
                return Ok(());
            }
            BatchStatus::DaPending { envelope_idx } => {
                let da_status = ctx
                    .da_provider
                    .check_da_status(batch.id(), envelope_idx)
                    .await?;
                debug!(?da_status, "checking da status");
                match da_status {
                    DaStatus::Pending => {
                        return Ok(());
                    }
                    DaStatus::Ready(da_refs) => {
                        debug!(batch_idx = target_idx, batch_id = ?batch.id(), "DA ready");

                        if let Err(e) = ctx.da_provider.confirm_da_complete(batch.id()).await {
                            warn!(
                                error = %e,
                                "failed to confirm DA complete; \
                                 leaving batch in DaPending and retrying next lifecycle tick"
                            );
                            return Ok(());
                        }

                        ctx.batch_storage
                            .update_batch_status(
                                batch.id(),
                                BatchStatus::DaComplete { da: da_refs },
                            )
                            .await?;

                        state.advance_da_complete(target_idx, batch.id());
                    }
                    DaStatus::NotRequested => {
                        warn!(
                            batch_idx = target_idx,
                            batch_id = ?batch.id(),
                            "Expected da operation to have been started. Retrying"
                        );

                        let new_envelope_idx = match ctx.da_provider.post_batch_da(batch.id()).await
                        {
                            Ok(envelope_idx) => envelope_idx,
                            Err(e) => {
                                warn!(
                                    batch_idx = target_idx,
                                    batch_id = ?batch.id(),
                                    error = %e,
                                    "failed to restart DA posting; will retry next lifecycle tick"
                                );
                                return Ok(());
                            }
                        };
                        ctx.batch_storage
                            .update_batch_status(
                                batch.id(),
                                BatchStatus::DaPending {
                                    envelope_idx: new_envelope_idx,
                                },
                            )
                            .await?;
                        return Ok(());
                    }
                    DaStatus::Failed { reason } => {
                        error!(
                            batch_idx = target_idx,
                            batch_id = ?batch.id(),
                            reason = %reason,
                            "CRITICAL: DA posting failed - manual intervention required. \
                             Batch is stuck in DaPending state."
                        );
                        return Ok(());
                    }
                };
            }
            BatchStatus::DaComplete { .. }
            | BatchStatus::ProofPending { .. }
            | BatchStatus::ProofReady { .. }
            | BatchStatus::Genesis => {
                state.advance_da_complete(target_idx, batch.id());
            }
        }
    }
}
