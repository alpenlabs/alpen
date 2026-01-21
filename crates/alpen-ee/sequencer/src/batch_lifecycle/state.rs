//! Batch lifecycle state management.

use alpen_ee_common::{require_latest_batch, BatchId, BatchStatus, BatchStorage, StorageError};

/// State for tracking batch lifecycle progress.
///
/// The lifecycle manager processes batches sequentially through their lifecycle states.
/// This struct tracks the "frontier" of each stage and any in-flight async operations.
///
/// Initialize using [`init_lifecycle_state`].
#[derive(Debug)]
pub struct BatchLifecycleState {
    /// Next batch index to post DA for.
    da_frontier_idx: u64,

    /// Next batch index to request proof for (after DA completion).
    proof_frontier_idx: u64,

    /// In-flight DA posting operation.
    pending_da: Option<PendingOperation>,

    /// In-flight proof generation.
    pending_proof: Option<PendingOperation>,
}

/// Pending operation tracking.
///
/// Tracks a batch that has an in-flight async operation (DA posting or proof generation).
/// Only stores the batch index and ID for reorg detection; actual data (txns, da_refs)
/// is read fresh from storage on each cycle.
#[derive(Debug, Clone)]
pub(crate) struct PendingOperation {
    pub idx: u64,
    pub batch_id: BatchId,
}
impl BatchLifecycleState {
    /// Get the proof frontier index.
    pub(crate) fn proof_frontier_idx(&self) -> u64 {
        self.proof_frontier_idx
    }

    /// Clear all pending operations.
    pub(crate) fn clear_pending_operations(&mut self) {
        self.pending_da = None;
        self.pending_proof = None;
    }

    /// Reset frontiers to start fresh from genesis.
    pub(crate) fn reset_frontiers(&mut self) {
        self.da_frontier_idx = 1; // Start after genesis
        self.proof_frontier_idx = 1;
    }

    /// Get the DA frontier index.
    pub(crate) fn da_frontier_idx(&self) -> u64 {
        self.da_frontier_idx
    }

    /// Check if there's a pending DA operation.
    pub(crate) fn pending_da(&self) -> Option<&PendingOperation> {
        self.pending_da.as_ref()
    }

    /// Check if there's a pending proof operation.
    pub(crate) fn pending_proof(&self) -> Option<&PendingOperation> {
        self.pending_proof.as_ref()
    }

    /// Take the pending DA (moves ownership).
    pub(crate) fn take_pending_da(&mut self) -> Option<PendingOperation> {
        self.pending_da.take()
    }

    /// Take the pending proof (moves ownership).
    pub(crate) fn take_pending_proof(&mut self) -> Option<PendingOperation> {
        self.pending_proof.take()
    }

    /// Set pending DA operation.
    pub(crate) fn set_pending_da(&mut self, pending: PendingOperation) {
        self.pending_da = Some(pending);
    }

    /// Set pending proof operation.
    pub(crate) fn set_pending_proof(&mut self, pending: PendingOperation) {
        self.pending_proof = Some(pending);
    }

    /// Advance DA frontier after posting.
    pub(crate) fn advance_da_frontier(&mut self) {
        self.da_frontier_idx += 1;
    }

    /// Advance proof frontier after completion.
    pub(crate) fn advance_proof_frontier(&mut self) {
        self.proof_frontier_idx += 1;
    }

    /// Check if we can start DA for the next batch.
    pub(crate) fn can_start_da(&self, latest_batch_idx: u64) -> bool {
        self.pending_da.is_none() && self.da_frontier_idx <= latest_batch_idx
    }

    /// Check if we can advance the proof frontier.
    pub(crate) fn can_advance_proof_frontier(&self, latest_batch_idx: u64) -> bool {
        self.pending_proof.is_none() && self.proof_frontier_idx <= latest_batch_idx
    }

    /// Create a new state for testing purposes.
    ///
    /// This constructor allows tests to create specific state configurations
    /// without going through storage initialization.
    #[cfg(test)]
    pub(crate) fn new_for_testing(
        da_frontier_idx: u64,
        proof_frontier_idx: u64,
        pending_da: Option<PendingOperation>,
        pending_proof: Option<PendingOperation>,
    ) -> Self {
        Self {
            da_frontier_idx,
            proof_frontier_idx,
            pending_da,
            pending_proof,
        }
    }
}

/// Initialize batch lifecycle state from storage.
///
/// This scans storage to find batches in intermediate states and determines
/// where to resume processing.
pub async fn init_lifecycle_state(
    storage: &impl BatchStorage,
) -> Result<BatchLifecycleState, StorageError> {
    let (latest_batch, _latest_status) = require_latest_batch(storage).await?;

    let latest_idx = latest_batch.idx();

    // Find the frontier positions by scanning backwards from latest batch
    let mut state = BatchLifecycleState {
        // target_batch_idx: latest_idx,
        da_frontier_idx: 1, // Start after genesis (idx 0)
        proof_frontier_idx: 1,
        pending_da: None,
        pending_proof: None,
    };

    // Scan batches to find where we are in the pipeline
    recover_from_storage(&mut state, storage, latest_idx).await?;

    Ok(state)
}

/// Recover state by scanning storage for batches in intermediate states.
pub(crate) async fn recover_from_storage(
    state: &mut BatchLifecycleState,
    storage: &impl BatchStorage,
    latest_idx: u64,
) -> Result<(), StorageError> {
    // Scan from idx 1 (skip genesis) to latest
    for idx in 1..=latest_idx {
        let Some((batch, status)) = storage.get_batch_by_idx(idx).await? else {
            break;
        };

        match status {
            BatchStatus::Sealed => {
                // This batch needs DA posting
                state.da_frontier_idx = idx;
                break;
            }
            BatchStatus::DaPending { .. } => {
                // Resume tracking this pending DA
                state.da_frontier_idx = idx + 1;
                state.proof_frontier_idx = idx;
                state.pending_da = Some(PendingOperation {
                    idx,
                    batch_id: batch.id(),
                });
                break;
            }
            BatchStatus::DaComplete { .. } | BatchStatus::ProofPending { .. } => {
                // Resume tracking pending proof
                state.da_frontier_idx = idx + 1;
                state.proof_frontier_idx = idx + 1;
                state.pending_proof = Some(PendingOperation {
                    idx,
                    batch_id: batch.id(),
                });
                break;
            }
            BatchStatus::ProofReady { .. } => {
                // This batch is complete, move frontiers past it
                state.da_frontier_idx = idx + 1;
                state.proof_frontier_idx = idx + 1;
            }
        }
    }

    Ok(())
}
