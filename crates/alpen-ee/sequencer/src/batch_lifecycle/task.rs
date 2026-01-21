//! Batch lifecycle task implementation.

use std::time::Duration;

use alpen_ee_common::{
    require_latest_batch, Batch, BatchDaProvider, BatchProver, BatchStatus, BatchStorage,
    ProofGenerationStatus,
};
use eyre::Result;
use tokio::time;
use tracing::{debug, error, warn};

use super::{
    ctx::BatchLifecycleCtx,
    reorg::{detect_reorg, handle_reorg, ReorgDetected},
    state::{BatchLifecycleState, PendingOperation},
};

/// Polling interval for checking DA confirmations and proof status.
const POLL_INTERVAL: Duration = Duration::from_secs(10);

/// Try to advance the DA frontier by one batch.
///
/// This checks the batch at da_frontier_idx and attempts to progress it:
/// - Sealed → Post DA → DaPending
/// - DaPending → Check DA status → (if confirmed) DaComplete → request proof → ProofPending
/// - DaComplete/ProofPending/ProofReady → Skip past (already processed)
async fn try_advance_da_frontier<D, P, S>(
    state: &mut BatchLifecycleState,
    latest_batch: &Batch,
    ctx: &BatchLifecycleCtx<D, P, S>,
) -> Result<()>
where
    D: BatchDaProvider,
    P: BatchProver,
    S: BatchStorage,
{
    // Can only advance if no pending DA and within target range
    if !state.can_start_da(latest_batch.idx()) {
        return Ok(());
    }

    let idx = state.da_frontier_idx();

    let Some((batch, status)) = ctx.batch_storage.get_batch_by_idx(idx).await? else {
        return Ok(()); // Batch doesn't exist yet
    };

    match status {
        BatchStatus::Sealed => {
            // Start DA posting
            debug!(batch_idx = idx, batch_id = ?batch.id(), "Posting DA");

            let txns = ctx.da_provider.post_batch_da(batch.id()).await?;

            ctx.batch_storage
                .update_batch_status(batch.id(), BatchStatus::DaPending { txns: txns.clone() })
                .await?;

            state.set_pending_da(PendingOperation {
                idx,
                batch_id: batch.id(),
            });
            state.advance_da_frontier();
        }

        BatchStatus::DaPending { txns } => {
            // Check if DA is confirmed
            if let Some(da_refs) = ctx.da_provider.check_da_status(&txns).await? {
                debug!(batch_idx = idx, batch_id = ?batch.id(), "DA confirmed, requesting proof");

                // Update to DaComplete
                ctx.batch_storage
                    .update_batch_status(
                        batch.id(),
                        BatchStatus::DaComplete {
                            da: da_refs.clone(),
                        },
                    )
                    .await?;

                // Immediately request proof
                ctx.prover.request_proof_generation(batch.id()).await?;

                // Update to ProofPending
                ctx.batch_storage
                    .update_batch_status(
                        batch.id(),
                        BatchStatus::ProofPending {
                            da: da_refs.clone(),
                        },
                    )
                    .await?;

                // Clear pending DA and track pending proof
                state.take_pending_da();
                state.set_pending_proof(PendingOperation {
                    idx,
                    batch_id: batch.id(),
                });
                state.advance_da_frontier();
            }
            // else: DA not confirmed yet, wait for next cycle
        }

        BatchStatus::DaComplete { .. }
        | BatchStatus::ProofPending { .. }
        | BatchStatus::ProofReady { .. } => {
            // Already past this stage, advance
            state.advance_da_frontier();
        }
    }

    Ok(())
}

/// Try to advance the proof frontier by one batch.
///
/// This checks the batch at proof_frontier_idx and attempts to progress it:
/// - Sealed/DaPending/DaComplete → Not ready for proof yet, no action
/// - ProofPending → Check proof status → (if ready) ProofReady
/// - ProofReady → Skip past (already complete)
async fn try_advance_proof_frontier<D, P, S>(
    state: &mut BatchLifecycleState,
    latest_batch: &Batch,
    ctx: &BatchLifecycleCtx<D, P, S>,
) -> Result<()>
where
    D: BatchDaProvider,
    P: BatchProver,
    S: BatchStorage,
{
    // Can only advance if no pending proof and within target range
    if !state.can_advance_proof_frontier(latest_batch.idx()) {
        return Ok(());
    }

    let idx = state.proof_frontier_idx();

    let Some((batch, status)) = ctx.batch_storage.get_batch_by_idx(idx).await? else {
        return Ok(()); // Batch doesn't exist yet
    };

    match status {
        BatchStatus::Sealed | BatchStatus::DaPending { .. } | BatchStatus::DaComplete { .. } => {
            // Not ready for proof checking yet, no action
            Ok(())
        }

        BatchStatus::ProofPending { da } => {
            // Check proof status
            match ctx.prover.check_proof_status(batch.id()).await? {
                ProofGenerationStatus::Ready { proof_id } => {
                    debug!(batch_idx = idx, batch_id = ?batch.id(), "Proof ready");

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
                    let _ = ctx.proof_ready_tx.send(batch.id());

                    // Clear pending and advance
                    state.take_pending_proof();
                    state.advance_proof_frontier();
                }

                ProofGenerationStatus::Failed { reason } => {
                    // CRITICAL: Manual intervention required
                    error!(
                        batch_idx = idx,
                        batch_id = ?batch.id(),
                        reason = %reason,
                        "CRITICAL: Proof generation failed - manual intervention required. \
                         Batch is stuck in ProofPending state."
                    );
                    // Keep pending_proof set, don't advance
                }

                ProofGenerationStatus::Pending | ProofGenerationStatus::NotStarted => {
                    // Still waiting, no action
                }
            }

            Ok(())
        }

        BatchStatus::ProofReady { .. } => {
            // Already complete, advance
            state.advance_proof_frontier();
            Ok(())
        }
    }
}

/// Main batch lifecycle task.
///
/// This task monitors sealed batches and manages their progression through
/// the lifecycle states: Sealed → DaPending → DaComplete → ProofPending → ProofReady.
///
/// Both event triggers (new batch notification, poll tick) trigger frontier
/// advancement checks.
pub(crate) async fn batch_lifecycle_task<D, P, S>(
    mut state: BatchLifecycleState,
    mut ctx: BatchLifecycleCtx<D, P, S>,
) where
    D: BatchDaProvider,
    P: BatchProver,
    S: BatchStorage,
{
    let mut poll_interval = time::interval(POLL_INTERVAL);

    loop {
        tokio::select! {
            // Branch 1: New sealed batch notification
            changed = ctx.sealed_batch_rx.changed() => {
                if changed.is_err() {
                    warn!("sealed_batch_rx channel closed; exiting");
                    return;
                }
            }

            // Branch 2: Poll interval tick
            _ = poll_interval.tick() => { }
        }

        // Get latest batch
        let latest_batch = match require_latest_batch(ctx.batch_storage.as_ref()).await {
            Ok((batch, _)) => batch,
            Err(e) => {
                error!(error = %e, "Failed to get latest batch");
                continue;
            }
        };

        // Detect and handle reorg
        let reorg = detect_reorg(&state, &latest_batch, ctx.batch_storage.as_ref()).await;
        match reorg {
            Ok(reorg) if !matches!(reorg, ReorgDetected::None) => {
                if let Err(e) =
                    handle_reorg(&mut state, &latest_batch, ctx.batch_storage.as_ref(), reorg).await
                {
                    error!(error = %e, "Failed to handle reorg");
                    continue;
                }
            }
            Err(e) => {
                error!(error = %e, "Failed to detect reorg");
                continue;
            }
            _ => {}
        }

        // Try to advance each frontier
        if let Err(e) = try_advance_da_frontier(&mut state, &latest_batch, &ctx).await {
            error!(error = %e, "Failed to advance DA frontier");
        }

        if let Err(e) = try_advance_proof_frontier(&mut state, &latest_batch, &ctx).await {
            error!(error = %e, "Failed to advance proof frontier");
        }
    }
}
