//! Reorg detection and handling for batch lifecycle.
//!
//! The batch_builder task owns storage mutations via `revert_batches()`. The lifecycle
//! manager must detect when its in-flight operations have been invalidated by comparing
//! batch identity (BatchId), not just index.

use alpen_ee_common::{Batch, BatchId, BatchStorage, StorageError};
use tracing::warn;

use super::state::{recover_from_storage, BatchLifecycleState};

/// Reorg detection result.
///
/// Indicates whether a reorg has been detected and what kind.
#[derive(Debug)]
pub(crate) enum ReorgDetected {
    /// No reorg detected.
    None,
    /// Target batch index moved backwards.
    TargetMovedBackward,
    /// Pending DA batch was removed from storage.
    PendingDaBatchRemoved,
    /// Pending DA batch was replaced with different content.
    PendingDaBatchReplaced {
        idx: u64,
        expected: BatchId,
        found: BatchId,
    },
    /// Pending proof batch was removed from storage.
    PendingProofBatchRemoved,
    /// Pending proof batch was replaced with different content.
    PendingProofBatchReplaced {
        idx: u64,
        expected: BatchId,
        found: BatchId,
    },
}

/// Check if any pending operations have been invalidated by a reorg.
///
/// A reorg is detected when:
/// 1. The target batch index moves backwards (batch_builder reverted batches), OR
/// 2. A pending operation references a batch that no longer exists in storage, OR
/// 3. A pending operation references a batch whose identity (BatchId) has changed
///
/// This is critical because batch_builder may revert and recreate batches at the
/// same index with different content (different last_block).
pub(crate) async fn detect_reorg(
    state: &BatchLifecycleState,
    latest_batch: &Batch,
    storage: &impl BatchStorage,
) -> Result<ReorgDetected, StorageError> {
    // Check 1: Target moved backwards (batches were reverted)
    if latest_batch.idx() < state.da_frontier_idx().saturating_sub(1) {
        return Ok(ReorgDetected::TargetMovedBackward);
    }

    // Check 2: Pending DA batch still valid
    if let Some(pending) = state.pending_da() {
        match storage.get_batch_by_idx(pending.idx).await? {
            None => {
                // Batch was removed
                return Ok(ReorgDetected::PendingDaBatchRemoved);
            }
            Some((batch, _)) if batch.id() != pending.batch_id => {
                // Batch was replaced with different content
                return Ok(ReorgDetected::PendingDaBatchReplaced {
                    idx: pending.idx,
                    expected: pending.batch_id,
                    found: batch.id(),
                });
            }
            Some(_) => { /* Batch still valid */ }
        }
    }

    // Check 3: Pending proof batch still valid
    if let Some(pending) = state.pending_proof() {
        match storage.get_batch_by_idx(pending.idx).await? {
            None => {
                return Ok(ReorgDetected::PendingProofBatchRemoved);
            }
            Some((batch, _)) if batch.id() != pending.batch_id => {
                return Ok(ReorgDetected::PendingProofBatchReplaced {
                    idx: pending.idx,
                    expected: pending.batch_id,
                    found: batch.id(),
                });
            }
            Some(_) => { /* Batch still valid */ }
        }
    }

    Ok(ReorgDetected::None)
}

/// Handle detected reorg by resetting state.
///
/// Clears all pending operations and re-scans storage to recover state from
/// the current valid batches.
pub(crate) async fn handle_reorg(
    state: &mut BatchLifecycleState,
    latest_batch: &Batch,
    storage: &impl BatchStorage,
    reason: ReorgDetected,
) -> Result<(), StorageError> {
    warn!(
        reason = ?reason,
        latest_idx = latest_batch.idx(),
        da_frontier = state.da_frontier_idx(),
        proof_frontier = state.proof_frontier_idx(),
        "Reorg detected, resetting lifecycle state"
    );

    // Clear all pending operations - they're now invalid
    state.clear_pending_operations();

    // Reset frontiers to start fresh from current storage state
    state.reset_frontiers();

    // Re-scan storage to find where we actually are
    let latest_idx = latest_batch.idx();
    recover_from_storage(state, storage, latest_idx).await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use alpen_ee_common::{BatchStatus, MockBatchStorage};
    use mockall::predicate::eq;

    use super::*;
    use crate::batch_lifecycle::{state::PendingOperation, test_utils::*};

    // ========================================================================
    // A. No Reorg Scenarios
    // ========================================================================

    #[tokio::test]
    async fn test_no_reorg_no_pending() {
        let state = BatchLifecycleState::new_for_testing(5, 5, None, None);
        let latest_batch = make_batch(10, 0, 10);
        let storage = MockBatchStorage::new();

        let result = detect_reorg(&state, &latest_batch, &storage).await.unwrap();
        assert!(matches!(result, ReorgDetected::None));
    }

    #[tokio::test]
    async fn test_no_reorg_pending_da_valid() {
        let pending_da = PendingOperation {
            idx: 3,
            batch_id: make_batch_id(2, 3),
        };
        let state = BatchLifecycleState::new_for_testing(4, 3, Some(pending_da.clone()), None);
        let latest_batch = make_batch(10, 0, 10);

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

        let result = detect_reorg(&state, &latest_batch, &storage).await.unwrap();
        assert!(matches!(result, ReorgDetected::None));
    }

    #[tokio::test]
    async fn test_no_reorg_pending_proof_valid() {
        let pending_proof = PendingOperation {
            idx: 3,
            batch_id: make_batch_id(2, 3),
        };
        let state = BatchLifecycleState::new_for_testing(4, 4, None, Some(pending_proof.clone()));
        let latest_batch = make_batch(10, 0, 10);

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

        let result = detect_reorg(&state, &latest_batch, &storage).await.unwrap();
        assert!(matches!(result, ReorgDetected::None));
    }

    #[tokio::test]
    async fn test_no_reorg_both_pending_valid() {
        let pending_da = PendingOperation {
            idx: 3,
            batch_id: make_batch_id(2, 3),
        };
        let pending_proof = PendingOperation {
            idx: 2,
            batch_id: make_batch_id(1, 2),
        };
        let state = BatchLifecycleState::new_for_testing(
            4,
            3,
            Some(pending_da.clone()),
            Some(pending_proof.clone()),
        );
        let latest_batch = make_batch(10, 0, 10);

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
            .expect_get_batch_by_idx()
            .with(eq(2))
            .times(1)
            .returning(|_| {
                Ok(Some((
                    make_batch(2, 1, 2),
                    BatchStatus::ProofPending {
                        da: vec![make_da_ref(1, 2)],
                    },
                )))
            });

        let result = detect_reorg(&state, &latest_batch, &storage).await.unwrap();
        assert!(matches!(result, ReorgDetected::None));
    }

    #[tokio::test]
    async fn test_no_reorg_target_advanced() {
        let state = BatchLifecycleState::new_for_testing(5, 5, None, None);
        let latest_batch = make_batch(20, 0, 20);
        let storage = MockBatchStorage::new();

        let result = detect_reorg(&state, &latest_batch, &storage).await.unwrap();
        assert!(matches!(result, ReorgDetected::None));
    }

    // ========================================================================
    // B. Target Moved Backward
    // ========================================================================

    #[tokio::test]
    async fn test_reorg_target_moved_backward_simple() {
        // DA frontier is at 5, but latest batch is at 3 (< 5 - 1 = 4)
        let state = BatchLifecycleState::new_for_testing(5, 5, None, None);
        let latest_batch = make_batch(3, 0, 3);
        let storage = MockBatchStorage::new();

        let result = detect_reorg(&state, &latest_batch, &storage).await.unwrap();
        assert!(matches!(result, ReorgDetected::TargetMovedBackward));
    }

    #[tokio::test]
    async fn test_reorg_target_moved_backward_to_genesis() {
        // DA frontier is at 10, reverted back to genesis (idx 0)
        let state = BatchLifecycleState::new_for_testing(10, 10, None, None);
        let latest_batch = make_genesis_batch(0); // Genesis batch
        let storage = MockBatchStorage::new();

        let result = detect_reorg(&state, &latest_batch, &storage).await.unwrap();
        assert!(matches!(result, ReorgDetected::TargetMovedBackward));
    }

    #[tokio::test]
    async fn test_reorg_target_moved_backward_with_pending() {
        // DA frontier is at 5, latest is at 2, and we have pending operations
        let pending_da = PendingOperation {
            idx: 4,
            batch_id: make_batch_id(3, 4),
        };
        let state = BatchLifecycleState::new_for_testing(5, 4, Some(pending_da), None);
        let latest_batch = make_batch(2, 0, 2);
        let storage = MockBatchStorage::new();

        let result = detect_reorg(&state, &latest_batch, &storage).await.unwrap();
        assert!(matches!(result, ReorgDetected::TargetMovedBackward));
    }

    // ========================================================================
    // C. Pending DA Invalidation
    // ========================================================================

    #[tokio::test]
    async fn test_reorg_pending_da_batch_removed() {
        let pending_da = PendingOperation {
            idx: 3,
            batch_id: make_batch_id(2, 3),
        };
        let state = BatchLifecycleState::new_for_testing(4, 3, Some(pending_da), None);
        let latest_batch = make_batch(10, 0, 10);

        let mut storage = MockBatchStorage::new();
        storage
            .expect_get_batch_by_idx()
            .with(eq(3))
            .times(1)
            .returning(|_| Ok(None)); // Batch no longer exists

        let result = detect_reorg(&state, &latest_batch, &storage).await.unwrap();
        assert!(matches!(result, ReorgDetected::PendingDaBatchRemoved));
    }

    #[tokio::test]
    async fn test_reorg_pending_da_batch_replaced_different_last_block() {
        let pending_da = PendingOperation {
            idx: 3,
            batch_id: make_batch_id(2, 3), // Expected: prev=2, last=3
        };
        let state = BatchLifecycleState::new_for_testing(4, 3, Some(pending_da.clone()), None);
        let latest_batch = make_batch(10, 0, 10);

        let mut storage = MockBatchStorage::new();
        storage
            .expect_get_batch_by_idx()
            .with(eq(3))
            .times(1)
            .returning(|_| {
                // Same idx, same prev_block, but different last_block (99 instead of 3)
                Ok(Some((
                    make_batch(3, 2, 99),
                    BatchStatus::DaPending {
                        txns: make_da_txns(3),
                    },
                )))
            });

        let result = detect_reorg(&state, &latest_batch, &storage).await.unwrap();
        match result {
            ReorgDetected::PendingDaBatchReplaced {
                idx,
                expected,
                found,
            } => {
                assert_eq!(idx, 3);
                assert_eq!(expected, pending_da.batch_id);
                assert_eq!(found, make_batch_id(2, 99));
            }
            _ => panic!("Expected PendingDaBatchReplaced, got {:?}", result),
        }
    }

    #[tokio::test]
    async fn test_reorg_pending_da_batch_replaced_different_prev_block() {
        let pending_da = PendingOperation {
            idx: 3,
            batch_id: make_batch_id(2, 3), // Expected: prev=2, last=3
        };
        let state = BatchLifecycleState::new_for_testing(4, 3, Some(pending_da.clone()), None);
        let latest_batch = make_batch(10, 0, 10);

        let mut storage = MockBatchStorage::new();
        storage
            .expect_get_batch_by_idx()
            .with(eq(3))
            .times(1)
            .returning(|_| {
                // Same idx, different prev_block (88), same last_block
                Ok(Some((
                    make_batch(3, 88, 3),
                    BatchStatus::DaPending {
                        txns: make_da_txns(3),
                    },
                )))
            });

        let result = detect_reorg(&state, &latest_batch, &storage).await.unwrap();
        match result {
            ReorgDetected::PendingDaBatchReplaced {
                idx,
                expected,
                found,
            } => {
                assert_eq!(idx, 3);
                assert_eq!(expected, pending_da.batch_id);
                assert_eq!(found, make_batch_id(88, 3));
            }
            _ => panic!("Expected PendingDaBatchReplaced, got {:?}", result),
        }
    }

    #[tokio::test]
    async fn test_reorg_pending_da_batch_replaced_both_different() {
        let pending_da = PendingOperation {
            idx: 3,
            batch_id: make_batch_id(2, 3),
        };
        let state = BatchLifecycleState::new_for_testing(4, 3, Some(pending_da.clone()), None);
        let latest_batch = make_batch(10, 0, 10);

        let mut storage = MockBatchStorage::new();
        storage
            .expect_get_batch_by_idx()
            .with(eq(3))
            .times(1)
            .returning(|_| {
                // Completely different BatchId (different prev and last)
                Ok(Some((
                    make_batch(3, 77, 88),
                    BatchStatus::DaPending {
                        txns: make_da_txns(3),
                    },
                )))
            });

        let result = detect_reorg(&state, &latest_batch, &storage).await.unwrap();
        match result {
            ReorgDetected::PendingDaBatchReplaced {
                idx,
                expected,
                found,
            } => {
                assert_eq!(idx, 3);
                assert_eq!(expected, pending_da.batch_id);
                assert_eq!(found, make_batch_id(77, 88));
            }
            _ => panic!("Expected PendingDaBatchReplaced, got {:?}", result),
        }
    }

    // ========================================================================
    // D. Pending Proof Invalidation
    // ========================================================================

    #[tokio::test]
    async fn test_reorg_pending_proof_batch_removed() {
        let pending_proof = PendingOperation {
            idx: 3,
            batch_id: make_batch_id(2, 3),
        };
        let state = BatchLifecycleState::new_for_testing(4, 4, None, Some(pending_proof));
        let latest_batch = make_batch(10, 0, 10);

        let mut storage = MockBatchStorage::new();
        storage
            .expect_get_batch_by_idx()
            .with(eq(3))
            .times(1)
            .returning(|_| Ok(None)); // Batch removed

        let result = detect_reorg(&state, &latest_batch, &storage).await.unwrap();
        assert!(matches!(result, ReorgDetected::PendingProofBatchRemoved));
    }

    #[tokio::test]
    async fn test_reorg_pending_proof_batch_replaced_different_last_block() {
        let pending_proof = PendingOperation {
            idx: 3,
            batch_id: make_batch_id(2, 3),
        };
        let state = BatchLifecycleState::new_for_testing(4, 4, None, Some(pending_proof.clone()));
        let latest_batch = make_batch(10, 0, 10);

        let mut storage = MockBatchStorage::new();
        storage
            .expect_get_batch_by_idx()
            .with(eq(3))
            .times(1)
            .returning(|_| {
                Ok(Some((
                    make_batch(3, 2, 99), // Different last_block
                    BatchStatus::ProofPending {
                        da: vec![make_da_ref(1, 3)],
                    },
                )))
            });

        let result = detect_reorg(&state, &latest_batch, &storage).await.unwrap();
        match result {
            ReorgDetected::PendingProofBatchReplaced {
                idx,
                expected,
                found,
            } => {
                assert_eq!(idx, 3);
                assert_eq!(expected, pending_proof.batch_id);
                assert_eq!(found, make_batch_id(2, 99));
            }
            _ => panic!("Expected PendingProofBatchReplaced, got {:?}", result),
        }
    }

    #[tokio::test]
    async fn test_reorg_pending_proof_batch_replaced_different_prev_block() {
        let pending_proof = PendingOperation {
            idx: 3,
            batch_id: make_batch_id(2, 3),
        };
        let state = BatchLifecycleState::new_for_testing(4, 4, None, Some(pending_proof.clone()));
        let latest_batch = make_batch(10, 0, 10);

        let mut storage = MockBatchStorage::new();
        storage
            .expect_get_batch_by_idx()
            .with(eq(3))
            .times(1)
            .returning(|_| {
                Ok(Some((
                    make_batch(3, 88, 3), // Different prev_block
                    BatchStatus::ProofPending {
                        da: vec![make_da_ref(1, 3)],
                    },
                )))
            });

        let result = detect_reorg(&state, &latest_batch, &storage).await.unwrap();
        match result {
            ReorgDetected::PendingProofBatchReplaced {
                idx,
                expected,
                found,
            } => {
                assert_eq!(idx, 3);
                assert_eq!(expected, pending_proof.batch_id);
                assert_eq!(found, make_batch_id(88, 3));
            }
            _ => panic!("Expected PendingProofBatchReplaced, got {:?}", result),
        }
    }

    #[tokio::test]
    async fn test_reorg_pending_proof_batch_replaced_both_different() {
        let pending_proof = PendingOperation {
            idx: 3,
            batch_id: make_batch_id(2, 3),
        };
        let state = BatchLifecycleState::new_for_testing(4, 4, None, Some(pending_proof.clone()));
        let latest_batch = make_batch(10, 0, 10);

        let mut storage = MockBatchStorage::new();
        storage
            .expect_get_batch_by_idx()
            .with(eq(3))
            .times(1)
            .returning(|_| {
                Ok(Some((
                    make_batch(3, 77, 88), // Completely different
                    BatchStatus::ProofPending {
                        da: vec![make_da_ref(1, 3)],
                    },
                )))
            });

        let result = detect_reorg(&state, &latest_batch, &storage).await.unwrap();
        match result {
            ReorgDetected::PendingProofBatchReplaced {
                idx,
                expected,
                found,
            } => {
                assert_eq!(idx, 3);
                assert_eq!(expected, pending_proof.batch_id);
                assert_eq!(found, make_batch_id(77, 88));
            }
            _ => panic!("Expected PendingProofBatchReplaced, got {:?}", result),
        }
    }

    // ========================================================================
    // E. Multiple Simultaneous Invalidations
    // ========================================================================

    #[tokio::test]
    async fn test_reorg_both_pending_da_invalidated_first() {
        // Both pending operations invalid, DA checked first and returns error
        let pending_da = PendingOperation {
            idx: 3,
            batch_id: make_batch_id(2, 3),
        };
        let pending_proof = PendingOperation {
            idx: 2,
            batch_id: make_batch_id(1, 2),
        };
        let state =
            BatchLifecycleState::new_for_testing(4, 3, Some(pending_da), Some(pending_proof));
        let latest_batch = make_batch(10, 0, 10);

        let mut storage = MockBatchStorage::new();
        storage
            .expect_get_batch_by_idx()
            .with(eq(3))
            .times(1)
            .returning(|_| Ok(None)); // DA batch removed

        let result = detect_reorg(&state, &latest_batch, &storage).await.unwrap();
        // Should detect DA invalidation first (checked before proof)
        assert!(matches!(result, ReorgDetected::PendingDaBatchRemoved));
    }

    #[tokio::test]
    async fn test_reorg_both_pending_proof_only_invalid() {
        // DA is valid, but proof is invalid
        let pending_da = PendingOperation {
            idx: 3,
            batch_id: make_batch_id(2, 3),
        };
        let pending_proof = PendingOperation {
            idx: 2,
            batch_id: make_batch_id(1, 2),
        };
        let state = BatchLifecycleState::new_for_testing(
            4,
            3,
            Some(pending_da.clone()),
            Some(pending_proof),
        );
        let latest_batch = make_batch(10, 0, 10);

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
            .expect_get_batch_by_idx()
            .with(eq(2))
            .times(1)
            .returning(|_| Ok(None)); // Proof batch removed

        let result = detect_reorg(&state, &latest_batch, &storage).await.unwrap();
        assert!(matches!(result, ReorgDetected::PendingProofBatchRemoved));
    }

    // ========================================================================
    // F. Reorg Handling Tests
    // ========================================================================

    #[tokio::test]
    async fn test_handle_reorg_clears_pending_operations() {
        let pending_da = PendingOperation {
            idx: 3,
            batch_id: make_batch_id(2, 3),
        };
        let pending_proof = PendingOperation {
            idx: 2,
            batch_id: make_batch_id(1, 2),
        };
        let mut state =
            BatchLifecycleState::new_for_testing(4, 3, Some(pending_da), Some(pending_proof));
        let latest_batch = make_batch(5, 0, 5);

        let mut storage = MockBatchStorage::new();
        // Mock recover_from_storage - just return Ok for this test
        storage.expect_get_batch_by_idx().returning(|idx| {
            if idx <= 5 {
                Ok(Some((
                    make_batch(idx, (idx - 1) as u8, idx as u8),
                    BatchStatus::ProofReady {
                        da: vec![make_da_ref(1, idx as u8)],
                        proof: test_proof_id(idx as u8),
                    },
                )))
            } else {
                Ok(None)
            }
        });

        handle_reorg(
            &mut state,
            &latest_batch,
            &storage,
            ReorgDetected::TargetMovedBackward,
        )
        .await
        .unwrap();

        // Verify pending operations were cleared
        assert!(state.pending_da().is_none());
        assert!(state.pending_proof().is_none());
    }

    #[tokio::test]
    async fn test_handle_reorg_calls_recovery() {
        let mut state = BatchLifecycleState::new_for_testing(10, 10, None, None);
        let latest_batch = make_batch(5, 0, 5);

        let mut storage = MockBatchStorage::new();
        // Mock minimal storage response to verify recover_from_storage is called
        storage.expect_get_batch_by_idx().returning(|_| Ok(None));

        handle_reorg(
            &mut state,
            &latest_batch,
            &storage,
            ReorgDetected::TargetMovedBackward,
        )
        .await
        .unwrap();

        // Verify recovery logic ran (frontiers reset to 1 when no batches found)
        assert_eq!(state.da_frontier_idx(), 1);
        assert_eq!(state.proof_frontier_idx(), 1);
    }
}
