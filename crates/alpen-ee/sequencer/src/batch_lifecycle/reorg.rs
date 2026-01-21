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
#[expect(dead_code, reason = "unused fields for logging")]
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
