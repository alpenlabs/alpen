//! Reorg detection and handling for batch lifecycle.
//!
//! The batch_builder task owns storage mutations via `revert_batches()`. The lifecycle
//! manager must detect when its in-flight operations have been invalidated by comparing
//! batch identity (BatchId), not just index.

use alpen_ee_common::{Batch, BatchStorage, StorageError};
use tracing::warn;

use super::state::{recover_from_storage, BatchLifecycleState};

/// Reorg detection result.
///
/// Indicates whether a reorg has been detected and what kind.
#[derive(Debug)]
#[allow(dead_code, clippy::allow_attributes, reason = "used in logging")]
pub(crate) enum ReorgDetected {
    /// No reorg detected.
    None,
    /// Target batch index moved backwards.
    TargetMovedBackward,
}

/// Check if the batch lifecycle state is inconsistent with storage.
///
/// A reorg is detected when:
/// 1. The latest batch index moved backwards (batch_builder reverted batches)
/// 2. A frontier is pointing at a batch that doesn't exist or has an unexpected status
///
/// This is critical because batch_builder may revert and recreate batches at the
/// same index with different content (different last_block).
pub(crate) async fn detect_reorg(
    state: &BatchLifecycleState,
    latest_batch: &Batch,
    _storage: &impl BatchStorage,
) -> Result<ReorgDetected, StorageError> {
    // Check if target moved backwards
    // Use da_post_frontier as it's the furthest frontier
    let last_da_posted_batch_idx = state.da_post_frontier().saturating_sub(1);
    if latest_batch.idx() < last_da_posted_batch_idx {
        return Ok(ReorgDetected::TargetMovedBackward);
    }

    // TODO: check using batch id

    Ok(ReorgDetected::None)
}

/// Handle detected reorg by resetting state.
///
/// Resets all frontiers and re-scans storage to recover state from the current valid batches.
pub(crate) async fn handle_reorg(
    state: &mut BatchLifecycleState,
    latest_batch: &Batch,
    storage: &impl BatchStorage,
    reason: ReorgDetected,
) -> Result<(), StorageError> {
    warn!(
        reason = ?reason,
        latest_idx = latest_batch.idx(),
        da_post_frontier = state.da_post_frontier(),
        da_confirm_frontier = state.da_confirm_frontier(),
        proof_request_frontier = state.proof_request_frontier(),
        proof_complete_frontier = state.proof_complete_frontier(),
        "Reorg detected, resetting lifecycle state"
    );

    // Reset all frontiers to start fresh from current storage state
    state.reset_frontiers();

    // Re-scan storage to find where we actually are
    let latest_idx = latest_batch.idx();
    recover_from_storage(state, storage, latest_idx).await?;

    Ok(())
}
