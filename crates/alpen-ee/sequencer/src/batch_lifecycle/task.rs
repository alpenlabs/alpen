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
    if let Err(e) = try_advance_da_pending(state, &latest_batch, ctx).await {
        error!(error = %e, "failed to advance da pending frontier");
    }
    if let Err(e) = try_advance_da_complete(state, &latest_batch, ctx).await {
        error!(error = %e, "failed to advance da complete frontier");
    }
    if let Err(e) = try_advance_proof_pending(state, &latest_batch, ctx).await {
        error!(error = %e, "failed to advance proof pending frontier");
    }
    if let Err(e) = try_advance_proof_ready(state, &latest_batch, ctx).await {
        error!(error = %e, "failed to advance proof ready frontier");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use alpen_ee_common::{
        DaStatus, InMemoryStorage, MockBatchDaProvider, MockBatchProver, ProofGenerationStatus,
    };
    use eyre::eyre;
    use tokio::sync::watch;

    use super::*;
    use crate::batch_lifecycle::{state::init_lifecycle_state, test_utils::*};

    struct MockedCtxBuilder {
        da_provider: MockBatchDaProvider,
        prover: MockBatchProver,
    }

    impl MockedCtxBuilder {
        fn build<S: BatchStorage>(
            self,
            batch_storage: Arc<S>,
        ) -> BatchLifecycleCtx<MockBatchDaProvider, MockBatchProver, S> {
            let da_provider = Arc::new(self.da_provider);
            let prover = Arc::new(self.prover);
            let (_sealed_batch_tx, sealed_batch_rx) = watch::channel(make_batch_id(0, 0));
            let (proof_ready_tx, _proof_ready_rx) = watch::channel(None);

            BatchLifecycleCtx {
                batch_storage,
                da_provider,
                prover,
                sealed_batch_rx,
                proof_ready_tx,
            }
        }

        fn new() -> Self {
            Self {
                da_provider: MockBatchDaProvider::new(),
                prover: MockBatchProver::new(),
            }
        }

        fn with_happy_mocks(mut self) -> Self {
            // All requests succeed immediately
            self.da_provider
                .expect_post_batch_da()
                .returning(|_| Ok(()));

            self.da_provider
                .expect_check_da_status()
                .returning(|_| Ok(DaStatus::Ready(vec![make_da_ref(1, 1)])));

            self.prover
                .expect_request_proof_generation()
                .returning(|_| Ok(()));

            self.prover.expect_check_proof_status().returning(|_| {
                Ok(ProofGenerationStatus::Ready {
                    proof_id: test_proof_id(1),
                })
            });

            self
        }

        fn with<F>(mut self, f: F) -> Self
        where
            F: FnOnce(&mut MockedCtxBuilder),
        {
            f(&mut self);
            self
        }
    }

    /// Happy path test: Single batch progresses from Sealed to ProofReady through all lifecycle
    /// stages.
    #[tokio::test]
    async fn test_batch_lifecycle_happy() {
        let storage = Arc::new(InMemoryStorage::new_empty());

        use TestBatchStatus::*;
        let batches = fill_storage(storage.as_ref(), &[ProofReady, Sealed]).await;

        let batch1_id = batches[1].id();
        let batch2_id = batches[2].id();

        let mut state = init_lifecycle_state(storage.as_ref()).await.unwrap();
        assert_eq!(state.da_pending().idx(), 1);
        assert_eq!(state.da_pending().id(), batch1_id);
        assert_eq!(state.da_complete().idx(), 1);
        assert_eq!(state.proof_pending().idx(), 1);
        assert_eq!(state.proof_ready().idx(), 1);

        // check that batches in storage are as expected initially
        assert_eq!(read_batch_statuses(&storage), [ProofReady, Sealed]);

        // Setup mocks
        let ctx = MockedCtxBuilder::new()
            .with_happy_mocks()
            .build(storage.clone());

        process_cycle(&mut state, &ctx).await.unwrap();

        // All steps are tried in order, and all requests succeed, so batch should complete whole
        // lifecycle in a single call. Frontiers now point to batch 1.
        assert_eq!(state.da_pending().idx(), 2);
        assert_eq!(state.da_pending().id(), batch2_id);
        assert_eq!(state.da_complete().idx(), 2);
        assert_eq!(state.da_complete().id(), batch2_id);
        assert_eq!(state.proof_pending().idx(), 2);
        assert_eq!(state.proof_pending().id(), batch2_id);
        assert_eq!(state.proof_ready().idx(), 2);
        assert_eq!(state.proof_ready().id(), batch2_id);

        // check that batch has been set to proof ready
        assert_eq!(read_batch_statuses(&storage), [ProofReady, ProofReady]);
    }

    #[tokio::test]
    async fn test_multi_batch_lifecycle_happy() {
        let storage = Arc::new(InMemoryStorage::new_empty());

        use TestBatchStatus::*;
        fill_storage(
            storage.as_ref(),
            &[
                ProofReady,
                ProofReady,
                ProofPending,
                ProofPending,
                DaComplete,
                DaComplete,
                DaPending,
                DaPending,
                Sealed,
                Sealed,
            ],
        )
        .await;

        // ensure that batches are as expected
        assert_eq!(
            read_batch_statuses(&storage),
            [
                ProofReady,
                ProofReady,
                ProofPending,
                ProofPending,
                DaComplete,
                DaComplete,
                DaPending,
                DaPending,
                Sealed,
                Sealed,
            ]
        );

        let mut state = init_lifecycle_state(storage.as_ref()).await.unwrap();

        // Setup mocks
        let ctx = MockedCtxBuilder::new()
            .with_happy_mocks()
            .build(storage.clone());

        for _ in 0..10 {
            process_cycle(&mut state, &ctx).await.unwrap();
        }

        // check that all batches have been processed
        assert!(read_batch_statuses(&storage)
            .iter()
            .all(|s| s == &ProofReady),);
    }

    #[tokio::test]
    async fn test_batch_lifecycle_happy_sequence() {
        let storage = Arc::new(InMemoryStorage::new_empty());

        use TestBatchStatus::*;
        fill_storage(storage.as_ref(), &[Sealed]).await;

        assert_eq!(read_batch_statuses(&storage), [Sealed]);

        let mut state = init_lifecycle_state(storage.as_ref()).await.unwrap();

        // cycle 1
        let ctx = MockedCtxBuilder::new()
            // check da status is pending
            .with(|b| {
                b.da_provider
                    .expect_post_batch_da()
                    .returning(|_| Err(eyre!("cannot post da right now")));
            })
            // everything else immediately succeeds
            .with_happy_mocks()
            .build(storage.clone());

        // state remains Sealed
        process_cycle(&mut state, &ctx).await.unwrap();
        assert_eq!(read_batch_statuses(&storage), [Sealed]);

        // cycle 2
        let ctx = MockedCtxBuilder::new()
            // check da status is pending
            .with(|b| {
                b.da_provider
                    .expect_check_da_status()
                    .returning(|_| Ok(DaStatus::Pending));
            })
            // everything else immediately succeeds
            .with_happy_mocks()
            .build(storage.clone());

        // state transitions from Sealed -> DaPending; cannot transition to DaComplete
        process_cycle(&mut state, &ctx).await.unwrap();
        assert_eq!(read_batch_statuses(&storage), [DaPending]);

        // state remains in DaPending
        process_cycle(&mut state, &ctx).await.unwrap();
        assert_eq!(read_batch_statuses(&storage), [DaPending]);

        // cycle 3
        let ctx = MockedCtxBuilder::new()
            // request proof generation fails
            .with(|b| {
                b.prover
                    .expect_request_proof_generation()
                    .returning(|_| Err(eyre!("cannot generate proof right now")));
            })
            // everything else immediately succeeds
            .with_happy_mocks()
            .build(storage.clone());

        // state transitions from DaPending -> DaComplete; cannot transition to ProofPending
        process_cycle(&mut state, &ctx).await.unwrap();
        assert_eq!(read_batch_statuses(&storage), [DaComplete]);

        // state remains in DaComplete
        process_cycle(&mut state, &ctx).await.unwrap();
        assert_eq!(read_batch_statuses(&storage), [DaComplete]);

        let ctx = MockedCtxBuilder::new()
            // check proof status is pending
            .with(|b| {
                b.prover
                    .expect_check_proof_status()
                    .returning(|_| Ok(ProofGenerationStatus::Pending));
            })
            // everything else immediately succeeds
            .with_happy_mocks()
            .build(storage.clone());

        // state transitions from DaComplete -> ProofPending; cannot transition to ProofReady
        process_cycle(&mut state, &ctx).await.unwrap();
        assert_eq!(read_batch_statuses(&storage), [ProofPending]);

        // state remains in ProofPending
        process_cycle(&mut state, &ctx).await.unwrap();
        assert_eq!(read_batch_statuses(&storage), [ProofPending]);

        let ctx = MockedCtxBuilder::new()
            // everything immediately succeeds
            .with_happy_mocks()
            .build(storage.clone());

        // state transitions to ProofReady
        process_cycle(&mut state, &ctx).await.unwrap();
        assert_eq!(read_batch_statuses(&storage), [ProofReady]);

        // state remains in ProofReady
        process_cycle(&mut state, &ctx).await.unwrap();
        assert_eq!(read_batch_statuses(&storage), [ProofReady]);
    }
}
