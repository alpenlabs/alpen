//! Batch lifecycle task implementation.

use std::time::Duration;

use alpen_ee_common::{require_latest_batch, BatchDaProvider, BatchProver, BatchStorage};
use eyre::Result;
use tokio::time;
use tracing::{error, warn};

use super::{
    ctx::BatchLifecycleCtx,
    lifecycle::{
        try_advance_da_complete, try_advance_da_pending, try_advance_proof_pending,
        try_advance_proof_ready,
    },
    reorg::{detect_reorg, handle_reorg, ReorgResult},
    state::BatchLifecycleState,
};

/// Polling interval for checking DA confirmations and proof status.
const POLL_INTERVAL: Duration = Duration::from_secs(10);

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
    let reorg = detect_reorg(state, ctx.batch_storage.as_ref()).await?;
    if !matches!(reorg, ReorgResult::None) {
        handle_reorg(state, &latest_batch, ctx.batch_storage.as_ref(), reorg).await?;
    }

    // Try to advance each frontier (order doesn't matter, they're independent)
    try_advance_da_pending(state, &latest_batch, ctx).await?;
    try_advance_da_complete(state, &latest_batch, ctx).await?;
    try_advance_proof_pending(state, &latest_batch, ctx).await?;
    try_advance_proof_ready(state, &latest_batch, ctx).await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use alpen_ee_common::{
        BatchId, BatchStatus, DaStatus, InMemoryStorage, MockBatchDaProvider, MockBatchProver,
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
        let genesis_id = genesis.id();
        storage.save_genesis_batch(genesis.clone()).await.unwrap();

        // Add batch 1 as Sealed
        let batch1 = make_batch(1, 0, 1);
        let batch1_id = batch1.id();
        storage.save_next_batch(batch1.clone()).await.unwrap();

        // Initialize state - all frontiers start at genesis (idx 0)
        let mut state = init_lifecycle_state(&*storage).await.unwrap();
        assert_eq!(state.da_pending().idx(), 0);
        assert_eq!(state.da_pending().id(), genesis_id);
        assert_eq!(state.da_complete().idx(), 0);
        assert_eq!(state.proof_pending().idx(), 0);
        assert_eq!(state.proof_ready().idx(), 0);

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

        // All steps are tried in order, and all requests succeed, so batch should complete whole
        // lifecycle in a single call. Frontiers now point to batch 1.
        assert_eq!(state.da_pending().idx(), 1);
        assert_eq!(state.da_pending().id(), batch1_id);
        assert_eq!(state.da_complete().idx(), 1);
        assert_eq!(state.da_complete().id(), batch1_id);
        assert_eq!(state.proof_pending().idx(), 1);
        assert_eq!(state.proof_pending().id(), batch1_id);
        assert_eq!(state.proof_ready().idx(), 1);
        assert_eq!(state.proof_ready().id(), batch1_id);

        let (_, status) = storage.get_batch_by_idx(1).await.unwrap().unwrap();
        assert!(matches!(status, BatchStatus::ProofReady { .. }));
    }
}
