//! Batch lifecycle task implementation.

use std::time::Duration;

use alpen_ee_common::{
    require_latest_batch, Batch, BatchDaProvider, BatchProver, BatchStatus, BatchStorage, DaStatus,
    ProofGenerationStatus,
};
use eyre::Result;
use tokio::time;
use tracing::{debug, error, warn};

use super::{
    ctx::BatchLifecycleCtx,
    reorg::{detect_reorg, handle_reorg, ReorgDetected},
    state::BatchLifecycleState,
};

/// Polling interval for checking DA confirmations and proof status.
const POLL_INTERVAL: Duration = Duration::from_secs(10);

/// Try to post DA for the next batch (Sealed → DaPending).
async fn try_post_da<D, P, S>(
    state: &mut BatchLifecycleState,
    latest_batch: &Batch,
    ctx: &BatchLifecycleCtx<D, P, S>,
) -> Result<()>
where
    D: BatchDaProvider,
    P: BatchProver,
    S: BatchStorage,
{
    let idx = state.da_post_frontier();

    // If we're past the latest batch, nothing to do
    if idx > latest_batch.idx() {
        return Ok(());
    }

    let Some((batch, status)) = ctx.batch_storage.get_batch_by_idx(idx).await? else {
        return Ok(()); // Batch doesn't exist yet
    };

    match status {
        BatchStatus::Sealed => {
            // Start DA posting. If this fails, we retry in the next cycle.
            debug!(batch_idx = idx, batch_id = ?batch.id(), "Posting DA");

            ctx.da_provider.post_batch_da(batch.id()).await?;

            ctx.batch_storage
                .update_batch_status(batch.id(), BatchStatus::DaPending)
                .await?;

            state.advance_da_post_frontier();
        }
        BatchStatus::DaPending
        | BatchStatus::DaComplete { .. }
        | BatchStatus::ProofPending { .. }
        | BatchStatus::ProofReady { .. }
        | BatchStatus::Genesis => {
            // Already past this stage, advance
            state.advance_da_post_frontier();
        }
    }

    Ok(())
}

/// Try to confirm DA for the next batch (DaPending → DaComplete).
async fn try_confirm_da<D, P, S>(
    state: &mut BatchLifecycleState,
    latest_batch: &Batch,
    ctx: &BatchLifecycleCtx<D, P, S>,
) -> Result<()>
where
    D: BatchDaProvider,
    P: BatchProver,
    S: BatchStorage,
{
    let idx = state.da_confirm_frontier();

    // If we're past the latest batch, nothing to do
    if idx > latest_batch.idx() {
        return Ok(());
    }

    let Some((batch, status)) = ctx.batch_storage.get_batch_by_idx(idx).await? else {
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
                    debug!(batch_idx = idx, batch_id = ?batch.id(), "DA confirmed");

                    ctx.batch_storage
                        .update_batch_status(batch.id(), BatchStatus::DaComplete { da: da_refs })
                        .await?;

                    state.advance_da_confirm_frontier();
                }
                DaStatus::NotRequested => {
                    // We've marked the batch as da pending, but da provider says da has not been
                    // requested. Try to re-request and hope for the best.
                    warn!(
                        batch_idx = idx,
                        batch_id = ?batch.id(),
                        "Expected da operation to have been started. Retrying"
                    );

                    ctx.da_provider.post_batch_da(batch.id()).await?;
                }
                DaStatus::Failed { reason } => {
                    // CRITICAL: Manual intervention required
                    error!(
                        batch_idx = idx,
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
            // Already past this stage, advance
            state.advance_da_confirm_frontier();
        }
    }

    Ok(())
}

/// Try to request proof for the next batch (DaComplete → ProofPending).
async fn try_request_proof<D, P, S>(
    state: &mut BatchLifecycleState,
    latest_batch: &Batch,
    ctx: &BatchLifecycleCtx<D, P, S>,
) -> Result<()>
where
    D: BatchDaProvider,
    P: BatchProver,
    S: BatchStorage,
{
    let idx = state.proof_request_frontier();

    // If we're past the latest batch, nothing to do
    if idx > latest_batch.idx() {
        return Ok(());
    }

    let Some((batch, status)) = ctx.batch_storage.get_batch_by_idx(idx).await? else {
        return Ok(()); // Batch doesn't exist yet
    };

    match status {
        BatchStatus::Sealed | BatchStatus::DaPending => {
            // Not ready, no action
        }
        BatchStatus::DaComplete { da } => {
            // Request proof generation. If this fails, we retry in the next cycle.
            debug!(batch_idx = idx, batch_id = ?batch.id(), "Requesting proof");

            ctx.prover.request_proof_generation(batch.id()).await?;

            ctx.batch_storage
                .update_batch_status(batch.id(), BatchStatus::ProofPending { da })
                .await?;

            state.advance_proof_request_frontier();
        }
        BatchStatus::ProofPending { .. }
        | BatchStatus::ProofReady { .. }
        | BatchStatus::Genesis => {
            // Already past this stage, advance
            state.advance_proof_request_frontier();
        }
    }

    Ok(())
}

/// Try to complete proof for the next batch (ProofPending → ProofReady).
async fn try_complete_proof<D, P, S>(
    state: &mut BatchLifecycleState,
    latest_batch: &Batch,
    ctx: &BatchLifecycleCtx<D, P, S>,
) -> Result<()>
where
    D: BatchDaProvider,
    P: BatchProver,
    S: BatchStorage,
{
    let idx = state.proof_complete_frontier();

    // If we're past the latest batch, nothing to do
    if idx > latest_batch.idx() {
        return Ok(());
    }

    let Some((batch, status)) = ctx.batch_storage.get_batch_by_idx(idx).await? else {
        return Ok(()); // Batch doesn't exist yet
    };

    match status {
        BatchStatus::Sealed | BatchStatus::DaPending | BatchStatus::DaComplete { .. } => {
            // Not ready, no action
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
                    let _ = ctx.proof_ready_tx.send(Some(batch.id()));

                    state.advance_proof_complete_frontier();
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
                    // Stay at frontier - manual intervention required
                }

                ProofGenerationStatus::Pending => {
                    // Still waiting, no action
                }

                ProofGenerationStatus::NotStarted => {
                    // We've marked the batch as proof pending, but prover says proof generation has
                    // not started. Try to re-request proof generation and hope
                    // for the best.
                    warn!(
                        batch_idx = idx,
                        batch_id = ?batch.id(),
                        "Expected proof generation to have been started. Retrying proof generation"
                    );

                    ctx.prover.request_proof_generation(batch.id()).await?;
                }
            }
        }
        BatchStatus::ProofReady { .. } | BatchStatus::Genesis => {
            // Already complete, advance
            state.advance_proof_complete_frontier();
        }
    }

    Ok(())
}

/// Process one cycle of the batch lifecycle.
///
/// Returns an error if a critical operation fails. The caller (task loop) decides
/// whether to continue or abort based on the error.
pub(crate) async fn process_cycle<D, P, S>(
    state: &mut BatchLifecycleState,
    ctx: &BatchLifecycleCtx<D, P, S>,
) -> Result<()>
where
    D: BatchDaProvider,
    P: BatchProver,
    S: BatchStorage,
{
    // Get latest batch
    let (latest_batch, _) = require_latest_batch(ctx.batch_storage.as_ref()).await?;

    // Detect and handle reorg
    let reorg = detect_reorg(state, &latest_batch, ctx.batch_storage.as_ref()).await?;
    if !matches!(reorg, ReorgDetected::None) {
        handle_reorg(state, &latest_batch, ctx.batch_storage.as_ref(), reorg).await?;
    }

    // Try to advance each frontier (order doesn't matter, they're independent)
    try_post_da(state, &latest_batch, ctx).await?;
    try_confirm_da(state, &latest_batch, ctx).await?;
    try_request_proof(state, &latest_batch, ctx).await?;
    try_complete_proof(state, &latest_batch, ctx).await?;

    Ok(())
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

        if let Err(e) = process_cycle(&mut state, &ctx).await {
            error!(error = %e, "batch lifecycle processing failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use alpen_ee_common::{
        BatchId, DaStatus, InMemoryStorage, MockBatchDaProvider, MockBatchProver,
        ProofGenerationStatus,
    };
    use tokio::sync::watch;

    use super::*;
    use crate::batch_lifecycle::{state::init_lifecycle_state, test_utils::*};

    /// Helper to create a test context
    fn _make_test_ctx<S: BatchStorage>(
        storage: Arc<S>,
        da_provider: MockBatchDaProvider,
        prover: MockBatchProver,
    ) -> BatchLifecycleCtx<MockBatchDaProvider, MockBatchProver, S> {
        let (_sealed_batch_tx, sealed_batch_rx) = watch::channel(make_batch_id(0, 0));
        let (proof_ready_tx, _proof_ready_rx) = watch::channel(None);

        BatchLifecycleCtx {
            batch_storage: storage,
            da_provider: Arc::new(da_provider),
            prover: Arc::new(prover),
            sealed_batch_rx,
            proof_ready_tx,
        }
    }

    /// Happy path test: Single batch progresses from Sealed to ProofReady through all lifecycle
    /// stages.
    #[tokio::test]
    async fn test_batch_lifecycle_happy() {
        let storage = Arc::new(InMemoryStorage::new());

        // Setup genesis
        let genesis = make_genesis_batch(0);
        storage.save_genesis_batch(genesis.clone()).await.unwrap();

        // Add batch 1 as Sealed
        let batch1 = make_batch(1, 0, 1);
        let batch1_id = batch1.id();
        storage.save_next_batch(batch1.clone()).await.unwrap();

        // Initialize state - all frontiers start at 1 (first batch after genesis)
        let mut state = init_lifecycle_state(&*storage).await.unwrap();
        assert_eq!(state.da_post_frontier(), 1);
        assert_eq!(state.da_confirm_frontier(), 1);
        assert_eq!(state.proof_request_frontier(), 1);
        assert_eq!(state.proof_complete_frontier(), 1);

        // Setup mocks
        let mut da_provider = MockBatchDaProvider::new();
        let mut prover = MockBatchProver::new();

        // All requests succeed immediately
        da_provider
            .expect_post_batch_da()
            .times(1)
            .withf(move |id: &BatchId| *id == batch1_id)
            .returning(|_| Ok(()));

        da_provider
            .expect_check_da_status()
            .withf(move |id: &BatchId| *id == batch1_id)
            .returning(|_| Ok(DaStatus::Ready(vec![make_da_ref(1, 1)])));

        prover
            .expect_request_proof_generation()
            .withf(move |id: &BatchId| *id == batch1_id)
            .returning(|_| Ok(()));

        prover
            .expect_check_proof_status()
            .withf(move |id: &BatchId| *id == batch1_id)
            .returning(|_| {
                Ok(ProofGenerationStatus::Ready {
                    proof_id: test_proof_id(1),
                })
            });

        let ctx = _make_test_ctx(storage.clone(), da_provider, prover);

        process_cycle(&mut state, &ctx).await.unwrap();

        // All steps are tried in order, and all requests succed, so batch should complete whole
        // lifecycle in a single call.
        assert_eq!(state.da_post_frontier(), 2);
        assert_eq!(state.da_confirm_frontier(), 2);
        assert_eq!(state.proof_request_frontier(), 2);
        assert_eq!(state.proof_complete_frontier(), 2);

        let (_, status) = storage.get_batch_by_idx(1).await.unwrap().unwrap();
        assert!(matches!(status, BatchStatus::ProofReady { .. }));
    }
}
