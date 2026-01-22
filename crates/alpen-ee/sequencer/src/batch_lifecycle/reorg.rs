//! Reorg detection and handling for batch lifecycle.
//!
//! The batch_builder task owns storage mutations via `revert_batches()`. The lifecycle
//! manager must detect when its in-flight operations have been invalidated by comparing
//! batch identity (BatchId), not just index.

use alpen_ee_common::{require_genesis_batch, Batch, BatchStorage, StorageError};
use tracing::warn;

use super::state::{recover_from_storage, BatchLifecycleState};

/// Result of reorg detection.
///
/// Indicates whether a reorg was detected and which frontier is the highest valid one.
#[derive(Debug)]
pub(crate) enum ReorgResult {
    /// No reorg detected, all frontiers are valid.
    None,
    /// da_pending frontier is invalid, but da_complete is valid.
    /// Reset da_pending to da_complete.
    ResetToDaComplete,
    /// da_complete frontier is invalid, but proof_pending is valid.
    /// Reset da_pending and da_complete to proof_pending.
    ResetToProofPending,
    /// proof_pending frontier is invalid, but proof_ready is valid.
    /// Reset all higher frontiers to proof_ready.
    ResetToProofReady,
    /// All frontiers are invalid, need full rescan from storage.
    FullRescan,
}

/// Check if the batch lifecycle state is consistent with storage.
///
/// A reorg is detected when a frontier's batch id doesn't match what's in storage
/// at that index. This can happen when batch_builder reverts and recreates batches
/// at the same index with different content.
///
/// The check proceeds from highest frontier (da_pending) to lowest (proof_ready),
/// returning the appropriate reset action based on which frontier is still valid.
pub(crate) async fn detect_reorg(
    state: &BatchLifecycleState,
    storage: &impl BatchStorage,
) -> Result<ReorgResult, StorageError> {
    // Check if da_pending frontier batch still exists and matches
    let da_pending_valid = storage
        .get_batch_by_idx(state.da_pending().idx())
        .await?
        .is_some_and(|(batch, _)| batch.id() == state.da_pending().id());

    if da_pending_valid {
        Ok(ReorgResult::None)
    } else {
        find_valid_frontier(state, storage).await
    }
}

/// Find the highest frontier that is still valid in storage.
async fn find_valid_frontier(
    state: &BatchLifecycleState,
    storage: &impl BatchStorage,
) -> Result<ReorgResult, StorageError> {
    // Check da_complete
    if let Some((batch, _)) = storage.get_batch_by_idx(state.da_complete().idx()).await? {
        if batch.id() == state.da_complete().id() {
            return Ok(ReorgResult::ResetToDaComplete);
        }
    }

    // Check proof_pending
    if let Some((batch, _)) = storage
        .get_batch_by_idx(state.proof_pending().idx())
        .await?
    {
        if batch.id() == state.proof_pending().id() {
            return Ok(ReorgResult::ResetToProofPending);
        }
    }

    // Check proof_ready
    if let Some((batch, _)) = storage.get_batch_by_idx(state.proof_ready().idx()).await? {
        if batch.id() == state.proof_ready().id() {
            return Ok(ReorgResult::ResetToProofReady);
        }
    }

    // No frontier matches, full rescan needed
    Ok(ReorgResult::FullRescan)
}

/// Handle detected reorg by resetting state appropriately.
///
/// Depending on the reorg result, either partially resets frontiers or does a full rescan.
pub(crate) async fn handle_reorg(
    state: &mut BatchLifecycleState,
    latest_batch: &Batch,
    storage: &impl BatchStorage,
    result: ReorgResult,
) -> Result<(), StorageError> {
    match result {
        ReorgResult::None => Ok(()),

        ReorgResult::ResetToDaComplete => {
            warn!(
                da_pending_idx = state.da_pending().idx(),
                da_complete_idx = state.da_complete().idx(),
                "Reorg detected: resetting da_pending to da_complete"
            );
            state.reset_to_da_complete();
            Ok(())
        }

        ReorgResult::ResetToProofPending => {
            warn!(
                da_pending_idx = state.da_pending().idx(),
                da_complete_idx = state.da_complete().idx(),
                proof_pending_idx = state.proof_pending().idx(),
                "Reorg detected: resetting to proof_pending"
            );
            state.reset_to_proof_pending();
            Ok(())
        }

        ReorgResult::ResetToProofReady => {
            warn!(
                da_pending_idx = state.da_pending().idx(),
                da_complete_idx = state.da_complete().idx(),
                proof_pending_idx = state.proof_pending().idx(),
                proof_ready_idx = state.proof_ready().idx(),
                "Reorg detected: resetting to proof_ready"
            );
            state.reset_to_proof_ready();
            Ok(())
        }

        ReorgResult::FullRescan => {
            warn!(
                latest_idx = latest_batch.idx(),
                da_pending_idx = state.da_pending().idx(),
                da_complete_idx = state.da_complete().idx(),
                proof_pending_idx = state.proof_pending().idx(),
                proof_ready_idx = state.proof_ready().idx(),
                "Reorg detected: full rescan required"
            );

            // Get genesis batch id for reset
            let (genesis_batch, _) = require_genesis_batch(storage).await?;
            let genesis_id = genesis_batch.id();

            state.reset_to_genesis(genesis_id);
            recover_from_storage(state, storage, latest_batch.idx()).await
        }
    }
}
