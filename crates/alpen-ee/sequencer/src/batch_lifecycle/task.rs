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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use alpen_ee_common::{MockBatchDaProvider, MockBatchProver, MockBatchStorage};
    use mockall::predicate::{always, eq};
    use tokio::sync::watch;

    use super::*;
    use crate::batch_lifecycle::test_utils::*;

    /// Helper to create a test context
    fn make_test_ctx(
        storage: MockBatchStorage,
        da_provider: MockBatchDaProvider,
        prover: MockBatchProver,
    ) -> BatchLifecycleCtx<MockBatchDaProvider, MockBatchProver, MockBatchStorage> {
        let (_sealed_batch_tx, sealed_batch_rx) = watch::channel(make_batch_id(0, 0));
        let (proof_ready_tx, _proof_ready_rx) = watch::channel(make_batch_id(0, 0));

        BatchLifecycleCtx {
            batch_storage: Arc::new(storage),
            da_provider: Arc::new(da_provider),
            prover: Arc::new(prover),
            sealed_batch_rx,
            proof_ready_tx,
        }
    }

    // ========================================================================
    // A. DA Frontier: Sealed → DaPending
    // ========================================================================

    #[tokio::test]
    async fn test_da_frontier_posts_da_for_sealed() {
        let mut state = BatchLifecycleState::new_for_testing(3, 3, None, None);
        let latest_batch = make_batch(5, 4, 5);

        let mut storage = MockBatchStorage::new();
        storage
            .expect_get_batch_by_idx()
            .with(eq(3))
            .times(1)
            .returning(|_| Ok(Some((make_batch(3, 2, 3), BatchStatus::Sealed))));
        storage
            .expect_update_batch_status()
            .with(eq(make_batch_id(2, 3)), always())
            .times(1)
            .returning(|_, _| Ok(()));

        let mut da_provider = MockBatchDaProvider::new();
        da_provider
            .expect_post_batch_da()
            .with(eq(make_batch_id(2, 3)))
            .times(1)
            .returning(|_| Ok(make_da_txns(3)));

        let prover = MockBatchProver::new();
        let ctx = make_test_ctx(storage, da_provider, prover);

        try_advance_da_frontier(&mut state, &latest_batch, &ctx)
            .await
            .unwrap();

        // Verify state changes
        assert_eq!(state.da_frontier_idx(), 4);
        assert!(state.pending_da().is_some());
        assert_eq!(state.pending_da().unwrap().idx, 3);
        assert_eq!(state.pending_da().unwrap().batch_id, make_batch_id(2, 3));
    }

    #[tokio::test]
    async fn test_da_frontier_sealed_da_post_fails() {
        let mut state = BatchLifecycleState::new_for_testing(3, 3, None, None);
        let latest_batch = make_batch(5, 4, 5);

        let mut storage = MockBatchStorage::new();
        storage
            .expect_get_batch_by_idx()
            .with(eq(3))
            .times(1)
            .returning(|_| Ok(Some((make_batch(3, 2, 3), BatchStatus::Sealed))));

        let mut da_provider = MockBatchDaProvider::new();
        da_provider
            .expect_post_batch_da()
            .with(eq(make_batch_id(2, 3)))
            .times(1)
            .returning(|_| Err(eyre::eyre!("DA post failed")));

        let prover = MockBatchProver::new();
        let ctx = make_test_ctx(storage, da_provider, prover);

        let result = try_advance_da_frontier(&mut state, &latest_batch, &ctx).await;

        // Should propagate error
        assert!(result.is_err());

        // Frontier NOT advanced
        assert_eq!(state.da_frontier_idx(), 3);
        assert!(state.pending_da().is_none());
    }

    #[tokio::test]
    async fn test_da_frontier_sealed_batch_not_exist() {
        let mut state = BatchLifecycleState::new_for_testing(3, 3, None, None);
        let latest_batch = make_batch(5, 4, 5);

        let mut storage = MockBatchStorage::new();
        storage
            .expect_get_batch_by_idx()
            .with(eq(3))
            .times(1)
            .returning(|_| Ok(None)); // Batch doesn't exist

        let da_provider = MockBatchDaProvider::new();
        let prover = MockBatchProver::new();
        let ctx = make_test_ctx(storage, da_provider, prover);

        try_advance_da_frontier(&mut state, &latest_batch, &ctx)
            .await
            .unwrap();

        // No action taken, frontier NOT advanced
        assert_eq!(state.da_frontier_idx(), 3);
        assert!(state.pending_da().is_none());
    }

    // ========================================================================
    // B. DA Frontier: DaPending → ProofPending
    // ========================================================================

    #[tokio::test]
    async fn test_da_frontier_da_confirmed() {
        // Frontier at 3, no pending (will process batch 3 which is DaPending)
        let mut state = BatchLifecycleState::new_for_testing(3, 3, None, None);
        let latest_batch = make_batch(5, 4, 5);

        let mut storage = MockBatchStorage::new();
        storage
            .expect_get_batch_by_idx()
            .with(eq(3))
            .times(1)
            .returning(|_| {
                Ok(Some((
                    make_batch(3, 2, 3),
                    BatchStatus::DaPending {
                        txns: make_da_txns(3),
                    },
                )))
            });
        storage
            .expect_update_batch_status()
            .times(2) // DaComplete, then ProofPending
            .returning(|_, _| Ok(()));

        let mut da_provider = MockBatchDaProvider::new();
        da_provider
            .expect_check_da_status()
            .with(eq(make_da_txns(3)))
            .times(1)
            .returning(|_| Ok(Some(vec![make_da_ref(1, 3)])));

        let mut prover = MockBatchProver::new();
        prover
            .expect_request_proof_generation()
            .with(eq(make_batch_id(2, 3)))
            .times(1)
            .returning(|_| Box::pin(async { Ok(()) }));

        let ctx = make_test_ctx(storage, da_provider, prover);

        try_advance_da_frontier(&mut state, &latest_batch, &ctx)
            .await
            .unwrap();

        // Verify state changes
        assert_eq!(state.da_frontier_idx(), 4);
        assert!(state.pending_da().is_none());
        assert!(state.pending_proof().is_some());
        assert_eq!(state.pending_proof().unwrap().idx, 3);
        assert_eq!(state.pending_proof().unwrap().batch_id, make_batch_id(2, 3));
    }

    #[tokio::test]
    async fn test_da_frontier_da_not_confirmed() {
        // Frontier at 3, no pending (will process batch 3 which is DaPending)
        let mut state = BatchLifecycleState::new_for_testing(3, 3, None, None);
        let latest_batch = make_batch(5, 4, 5);

        let mut storage = MockBatchStorage::new();
        storage
            .expect_get_batch_by_idx()
            .with(eq(3))
            .times(1)
            .returning(|_| {
                Ok(Some((
                    make_batch(3, 2, 3),
                    BatchStatus::DaPending {
                        txns: make_da_txns(3),
                    },
                )))
            });

        let mut da_provider = MockBatchDaProvider::new();
        da_provider
            .expect_check_da_status()
            .with(eq(make_da_txns(3)))
            .times(1)
            .returning(|_| Ok(None)); // Not confirmed yet

        let prover = MockBatchProver::new();
        let ctx = make_test_ctx(storage, da_provider, prover);

        try_advance_da_frontier(&mut state, &latest_batch, &ctx)
            .await
            .unwrap();

        // No state changes - DA not confirmed yet, so still in DaPending
        // But it DID set pending_da when first posting
        assert_eq!(state.da_frontier_idx(), 3); // Not advanced
        assert!(state.pending_da().is_none()); // No pending set since DA didn't start from Sealed
        assert!(state.pending_proof().is_none());
    }

    #[tokio::test]
    async fn test_da_frontier_da_check_fails() {
        // Frontier at 3, no pending (will process batch 3 which is DaPending)
        let mut state = BatchLifecycleState::new_for_testing(3, 3, None, None);
        let latest_batch = make_batch(5, 4, 5);

        let mut storage = MockBatchStorage::new();
        storage
            .expect_get_batch_by_idx()
            .with(eq(3))
            .times(1)
            .returning(|_| {
                Ok(Some((
                    make_batch(3, 2, 3),
                    BatchStatus::DaPending {
                        txns: make_da_txns(3),
                    },
                )))
            });

        let mut da_provider = MockBatchDaProvider::new();
        da_provider
            .expect_check_da_status()
            .with(eq(make_da_txns(3)))
            .times(1)
            .returning(|_| Err(eyre::eyre!("DA check failed")));

        let prover = MockBatchProver::new();
        let ctx = make_test_ctx(storage, da_provider, prover);

        let result = try_advance_da_frontier(&mut state, &latest_batch, &ctx).await;

        // Should propagate error
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_da_frontier_proof_request_fails() {
        // Frontier at 3, no pending (will process batch 3 which is DaPending)
        let mut state = BatchLifecycleState::new_for_testing(3, 3, None, None);
        let latest_batch = make_batch(5, 4, 5);

        let mut storage = MockBatchStorage::new();
        storage
            .expect_get_batch_by_idx()
            .with(eq(3))
            .times(1)
            .returning(|_| {
                Ok(Some((
                    make_batch(3, 2, 3),
                    BatchStatus::DaPending {
                        txns: make_da_txns(3),
                    },
                )))
            });
        storage
            .expect_update_batch_status()
            .times(1) // Only DaComplete update succeeds
            .returning(|_, _| Ok(()));

        let mut da_provider = MockBatchDaProvider::new();
        da_provider
            .expect_check_da_status()
            .with(eq(make_da_txns(3)))
            .times(1)
            .returning(|_| Ok(Some(vec![make_da_ref(1, 3)])));

        let mut prover = MockBatchProver::new();
        prover
            .expect_request_proof_generation()
            .with(eq(make_batch_id(2, 3)))
            .times(1)
            .returning(|_| Box::pin(async { Err(eyre::eyre!("Proof request failed")) }));

        let ctx = make_test_ctx(storage, da_provider, prover);

        let result = try_advance_da_frontier(&mut state, &latest_batch, &ctx).await;

        // Should propagate error
        assert!(result.is_err());
    }

    // ========================================================================
    // C. DA Frontier: Skip Already-Complete
    // ========================================================================

    #[tokio::test]
    async fn test_da_frontier_skip_da_complete() {
        let mut state = BatchLifecycleState::new_for_testing(3, 3, None, None);
        let latest_batch = make_batch(5, 4, 5);

        let mut storage = MockBatchStorage::new();
        storage
            .expect_get_batch_by_idx()
            .with(eq(3))
            .times(1)
            .returning(|_| {
                Ok(Some((
                    make_batch(3, 2, 3),
                    BatchStatus::DaComplete {
                        da: vec![make_da_ref(1, 3)],
                    },
                )))
            });

        let da_provider = MockBatchDaProvider::new();
        let prover = MockBatchProver::new();
        let ctx = make_test_ctx(storage, da_provider, prover);

        try_advance_da_frontier(&mut state, &latest_batch, &ctx)
            .await
            .unwrap();

        // Should advance past DaComplete
        assert_eq!(state.da_frontier_idx(), 4);
        assert!(state.pending_da().is_none());
        assert!(state.pending_proof().is_none());
    }

    #[tokio::test]
    async fn test_da_frontier_skip_proof_ready() {
        let mut state = BatchLifecycleState::new_for_testing(3, 3, None, None);
        let latest_batch = make_batch(5, 4, 5);

        let mut storage = MockBatchStorage::new();
        storage
            .expect_get_batch_by_idx()
            .with(eq(3))
            .times(1)
            .returning(|_| {
                Ok(Some((
                    make_batch(3, 2, 3),
                    BatchStatus::ProofReady {
                        da: vec![make_da_ref(1, 3)],
                        proof: test_proof_id(3),
                    },
                )))
            });

        let da_provider = MockBatchDaProvider::new();
        let prover = MockBatchProver::new();
        let ctx = make_test_ctx(storage, da_provider, prover);

        try_advance_da_frontier(&mut state, &latest_batch, &ctx)
            .await
            .unwrap();

        // Should advance past ProofReady
        assert_eq!(state.da_frontier_idx(), 4);
        assert!(state.pending_da().is_none());
        assert!(state.pending_proof().is_none());
    }

    // ========================================================================
    // D. Proof Frontier: ProofPending → ProofReady
    // ========================================================================

    #[tokio::test]
    async fn test_proof_frontier_proof_ready() {
        // Frontier at 3, no pending (will process batch 3 which is ProofPending)
        let mut state = BatchLifecycleState::new_for_testing(3, 3, None, None);
        let latest_batch = make_batch(5, 4, 5);

        let mut storage = MockBatchStorage::new();
        storage
            .expect_get_batch_by_idx()
            .with(eq(3))
            .times(1)
            .returning(|_| {
                Ok(Some((
                    make_batch(3, 2, 3),
                    BatchStatus::ProofPending {
                        da: vec![make_da_ref(1, 3)],
                    },
                )))
            });
        storage
            .expect_update_batch_status()
            .times(1)
            .returning(|_, _| Ok(()));

        let mut prover = MockBatchProver::new();
        prover
            .expect_check_proof_status()
            .with(eq(make_batch_id(2, 3)))
            .times(1)
            .returning(|_| {
                Box::pin(async {
                    Ok(ProofGenerationStatus::Ready {
                        proof_id: test_proof_id(3),
                    })
                })
            });

        let da_provider = MockBatchDaProvider::new();
        let ctx = make_test_ctx(storage, da_provider, prover);

        try_advance_proof_frontier(&mut state, &latest_batch, &ctx)
            .await
            .unwrap();

        // Verify state changes
        assert_eq!(state.proof_frontier_idx(), 4);
        assert!(state.pending_proof().is_none());
        // DA frontier shouldn't change
        assert_eq!(state.da_frontier_idx(), 3);
    }

    #[tokio::test]
    async fn test_proof_frontier_proof_pending() {
        // Frontier at 3, no pending (will process batch 3 which is ProofPending)
        let mut state = BatchLifecycleState::new_for_testing(3, 3, None, None);
        let latest_batch = make_batch(5, 4, 5);

        let mut storage = MockBatchStorage::new();
        storage
            .expect_get_batch_by_idx()
            .with(eq(3))
            .times(1)
            .returning(|_| {
                Ok(Some((
                    make_batch(3, 2, 3),
                    BatchStatus::ProofPending {
                        da: vec![make_da_ref(1, 3)],
                    },
                )))
            });

        let mut prover = MockBatchProver::new();
        prover
            .expect_check_proof_status()
            .with(eq(make_batch_id(2, 3)))
            .times(1)
            .returning(|_| Box::pin(async { Ok(ProofGenerationStatus::Pending) }));

        let da_provider = MockBatchDaProvider::new();
        let ctx = make_test_ctx(storage, da_provider, prover);

        try_advance_proof_frontier(&mut state, &latest_batch, &ctx)
            .await
            .unwrap();

        // No state changes - proof still pending
        assert_eq!(state.proof_frontier_idx(), 3); // Not advanced
        assert!(state.pending_proof().is_none()); // Not set because wasn't transitioned from
                                                  // earlier state
    }

    #[tokio::test]
    async fn test_proof_frontier_proof_failed() {
        // Frontier at 3, no pending (will process batch 3 which is ProofPending)
        let mut state = BatchLifecycleState::new_for_testing(3, 3, None, None);
        let latest_batch = make_batch(5, 4, 5);

        let mut storage = MockBatchStorage::new();
        storage
            .expect_get_batch_by_idx()
            .with(eq(3))
            .times(1)
            .returning(|_| {
                Ok(Some((
                    make_batch(3, 2, 3),
                    BatchStatus::ProofPending {
                        da: vec![make_da_ref(1, 3)],
                    },
                )))
            });

        let mut prover = MockBatchProver::new();
        prover
            .expect_check_proof_status()
            .with(eq(make_batch_id(2, 3)))
            .times(1)
            .returning(|_| {
                Box::pin(async {
                    Ok(ProofGenerationStatus::Failed {
                        reason: "Test failure".to_string(),
                    })
                })
            });

        let da_provider = MockBatchDaProvider::new();
        let ctx = make_test_ctx(storage, da_provider, prover);

        try_advance_proof_frontier(&mut state, &latest_batch, &ctx)
            .await
            .unwrap();

        // Doesn't advance, keeps state unchanged
        assert_eq!(state.proof_frontier_idx(), 3);
        assert!(state.pending_proof().is_none()); // Not set because proof failed (see
                                                  // implementation)
    }

    #[tokio::test]
    async fn test_proof_frontier_proof_check_fails() {
        // Frontier at 3, no pending (will process batch 3 which is ProofPending)
        let mut state = BatchLifecycleState::new_for_testing(3, 3, None, None);
        let latest_batch = make_batch(5, 4, 5);

        let mut storage = MockBatchStorage::new();
        storage
            .expect_get_batch_by_idx()
            .with(eq(3))
            .times(1)
            .returning(|_| {
                Ok(Some((
                    make_batch(3, 2, 3),
                    BatchStatus::ProofPending {
                        da: vec![make_da_ref(1, 3)],
                    },
                )))
            });

        let mut prover = MockBatchProver::new();
        prover
            .expect_check_proof_status()
            .with(eq(make_batch_id(2, 3)))
            .times(1)
            .returning(|_| Box::pin(async { Err(eyre::eyre!("Proof check failed")) }));

        let da_provider = MockBatchDaProvider::new();
        let ctx = make_test_ctx(storage, da_provider, prover);

        let result = try_advance_proof_frontier(&mut state, &latest_batch, &ctx).await;

        // Should propagate error
        assert!(result.is_err());
    }

    // ========================================================================
    // E. Proof Frontier: Not Ready / Skip
    // ========================================================================

    #[tokio::test]
    async fn test_proof_frontier_batch_sealed() {
        let mut state = BatchLifecycleState::new_for_testing(3, 3, None, None);
        let latest_batch = make_batch(5, 4, 5);

        let mut storage = MockBatchStorage::new();
        storage
            .expect_get_batch_by_idx()
            .with(eq(3))
            .times(1)
            .returning(|_| Ok(Some((make_batch(3, 2, 3), BatchStatus::Sealed))));

        let da_provider = MockBatchDaProvider::new();
        let prover = MockBatchProver::new();
        let ctx = make_test_ctx(storage, da_provider, prover);

        try_advance_proof_frontier(&mut state, &latest_batch, &ctx)
            .await
            .unwrap();

        // No action, frontier NOT advanced
        assert_eq!(state.proof_frontier_idx(), 3);
        assert!(state.pending_proof().is_none());
    }

    #[tokio::test]
    async fn test_proof_frontier_batch_da_pending() {
        let mut state = BatchLifecycleState::new_for_testing(3, 3, None, None);
        let latest_batch = make_batch(5, 4, 5);

        let mut storage = MockBatchStorage::new();
        storage
            .expect_get_batch_by_idx()
            .with(eq(3))
            .times(1)
            .returning(|_| {
                Ok(Some((
                    make_batch(3, 2, 3),
                    BatchStatus::DaPending {
                        txns: make_da_txns(3),
                    },
                )))
            });

        let da_provider = MockBatchDaProvider::new();
        let prover = MockBatchProver::new();
        let ctx = make_test_ctx(storage, da_provider, prover);

        try_advance_proof_frontier(&mut state, &latest_batch, &ctx)
            .await
            .unwrap();

        // No action, frontier NOT advanced
        assert_eq!(state.proof_frontier_idx(), 3);
        assert!(state.pending_proof().is_none());
    }

    #[tokio::test]
    async fn test_proof_frontier_skip_proof_ready() {
        let mut state = BatchLifecycleState::new_for_testing(3, 3, None, None);
        let latest_batch = make_batch(5, 4, 5);

        let mut storage = MockBatchStorage::new();
        storage
            .expect_get_batch_by_idx()
            .with(eq(3))
            .times(1)
            .returning(|_| {
                Ok(Some((
                    make_batch(3, 2, 3),
                    BatchStatus::ProofReady {
                        da: vec![make_da_ref(1, 3)],
                        proof: test_proof_id(3),
                    },
                )))
            });

        let da_provider = MockBatchDaProvider::new();
        let prover = MockBatchProver::new();
        let ctx = make_test_ctx(storage, da_provider, prover);

        try_advance_proof_frontier(&mut state, &latest_batch, &ctx)
            .await
            .unwrap();

        // Should advance past ProofReady
        assert_eq!(state.proof_frontier_idx(), 4);
        assert!(state.pending_proof().is_none());
    }
}
