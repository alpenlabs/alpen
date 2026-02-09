//! Batch lifecycle state management.

use alpen_ee_common::{require_genesis_batch, BatchId, BatchStatus, BatchStorage, StorageError};

/// A frontier tracks the latest batch that has reached a particular status.
///
/// Each frontier stores both the batch index and its BatchId, allowing us to detect
/// reorgs by comparing the stored id against what's actually in storage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Frontier {
    /// Batch index of the latest batch at this status.
    idx: u64,
    /// BatchId of the batch at this index.
    id: BatchId,
}

impl Frontier {
    /// Create a new frontier.
    pub(crate) fn new(idx: u64, id: BatchId) -> Self {
        Self { idx, id }
    }

    /// Create a genesis frontier (idx 0 with the given genesis batch id).
    pub(crate) fn genesis(genesis_id: BatchId) -> Self {
        Self {
            idx: 0,
            id: genesis_id,
        }
    }

    /// Get the batch index.
    pub(crate) fn idx(&self) -> u64 {
        self.idx
    }

    /// Get the batch id.
    pub(crate) fn id(&self) -> BatchId {
        self.id
    }
}

/// State for tracking batch lifecycle progress.
///
/// The lifecycle manager processes batches sequentially through their lifecycle states.
/// Each frontier tracks the latest batch that has reached that status (by both idx and id).
///
/// Batch Lifecycle States:
/// Genesis → Sealed → DaPending → DaComplete → ProofPending → ProofReady
///
/// 4 Frontiers (each tracks the latest batch at that status or beyond):
/// 1. da_pending     - latest batch with DA posted (status >= DaPending)
/// 2. da_complete    - latest batch with DA confirmed (status >= DaComplete)
/// 3. proof_pending  - latest batch with proof requested (status >= ProofPending)
/// 4. proof_ready    - latest batch with proof complete (status == ProofReady)
///
/// To process the next batch for a transition, use `frontier.idx + 1`.
///
/// Invariant: `proof_ready.idx <= proof_pending.idx <= da_complete.idx <= da_pending.idx`
///
/// Initialize using [`init_lifecycle_state`].
#[derive(Debug)]
pub struct BatchLifecycleState {
    /// Latest batch with DA posted (status >= DaPending).
    da_pending: Frontier,

    /// Latest batch with DA confirmed (status >= DaComplete).
    da_complete: Frontier,

    /// Latest batch with proof requested (status >= ProofPending).
    proof_pending: Frontier,

    /// Latest batch with proof complete (status == ProofReady).
    proof_ready: Frontier,
}

impl BatchLifecycleState {
    /// Create a new state with all frontiers at genesis.
    fn new_at_genesis(genesis_id: BatchId) -> Self {
        let genesis = Frontier::genesis(genesis_id);
        Self {
            da_pending: genesis,
            da_complete: genesis,
            proof_pending: genesis,
            proof_ready: genesis,
        }
    }

    /// Get the DA pending frontier.
    pub(crate) fn da_pending(&self) -> &Frontier {
        &self.da_pending
    }

    /// Get the DA complete frontier.
    pub(crate) fn da_complete(&self) -> &Frontier {
        &self.da_complete
    }

    /// Get the proof pending frontier.
    pub(crate) fn proof_pending(&self) -> &Frontier {
        &self.proof_pending
    }

    /// Get the proof ready frontier.
    pub(crate) fn proof_ready(&self) -> &Frontier {
        &self.proof_ready
    }

    /// Advance DA pending frontier to a new batch.
    pub(crate) fn advance_da_pending(&mut self, idx: u64, id: BatchId) {
        self.da_pending = Frontier::new(idx, id);
    }

    /// Advance DA complete frontier to a new batch.
    pub(crate) fn advance_da_complete(&mut self, idx: u64, id: BatchId) {
        self.da_complete = Frontier::new(idx, id);
    }

    /// Advance proof pending frontier to a new batch.
    pub(crate) fn advance_proof_pending(&mut self, idx: u64, id: BatchId) {
        self.proof_pending = Frontier::new(idx, id);
    }

    /// Advance proof ready frontier to a new batch.
    pub(crate) fn advance_proof_ready(&mut self, idx: u64, id: BatchId) {
        self.proof_ready = Frontier::new(idx, id);
    }

    /// Reset all frontiers to genesis state.
    pub(crate) fn reset_to_genesis(&mut self, genesis_id: BatchId) {
        let genesis = Frontier::genesis(genesis_id);
        self.da_pending = genesis;
        self.da_complete = genesis;
        self.proof_pending = genesis;
        self.proof_ready = genesis;
    }

    /// Reset da_pending to match da_complete.
    pub(crate) fn reset_to_da_complete(&mut self) {
        self.da_pending = self.da_complete;
    }

    /// Reset da_pending and da_complete to match proof_pending.
    pub(crate) fn reset_to_proof_pending(&mut self) {
        self.da_pending = self.proof_pending;
        self.da_complete = self.proof_pending;
    }

    /// Reset all higher frontiers to match proof_ready.
    pub(crate) fn reset_to_proof_ready(&mut self) {
        self.da_pending = self.proof_ready;
        self.da_complete = self.proof_ready;
        self.proof_pending = self.proof_ready;
    }
}

/// Initialize batch lifecycle state from storage.
///
/// This scans storage from latest to earliest to find batches at each status level
/// and determines where to resume processing.
pub async fn init_lifecycle_state(
    storage: &impl BatchStorage,
) -> Result<BatchLifecycleState, StorageError> {
    let (genesis_batch, _) = require_genesis_batch(storage).await?;
    let genesis_id = genesis_batch.id();

    // Start with all frontiers at genesis
    let mut state = BatchLifecycleState::new_at_genesis(genesis_id);

    // Get the latest batch to know the scan range
    let Some((latest_batch, _)) = storage.get_latest_batch().await? else {
        // Only genesis exists, return genesis state
        return Ok(state);
    };

    let latest_idx = latest_batch.idx();
    if latest_idx == 0 {
        // Only genesis exists
        return Ok(state);
    }

    // Scan batches to find where we are in the pipeline
    recover_from_storage(&mut state, storage, latest_idx).await?;

    Ok(state)
}

/// Recover state by scanning storage from latest to earliest.
///
/// Frontiers track the latest batch at each status level. We scan from latest
/// to earliest to find the highest batch at each status.
pub(crate) async fn recover_from_storage(
    state: &mut BatchLifecycleState,
    storage: &impl BatchStorage,
    latest_idx: u64,
) -> Result<(), StorageError> {
    // Track which frontiers we've found (we want the highest idx for each)
    let mut found_da_pending = false;
    let mut found_da_complete = false;
    let mut found_proof_pending = false;
    let mut found_proof_ready = false;

    // Scan from latest to earliest (skip genesis at idx 0)
    for idx in (1..=latest_idx).rev() {
        let Some((batch, status)) = storage.get_batch_by_idx(idx).await? else {
            continue;
        };

        let frontier = Frontier::new(idx, batch.id());

        // Check what status level this batch has reached
        let at_least_da_pending = matches!(
            status,
            BatchStatus::DaPending { .. }
                | BatchStatus::DaComplete { .. }
                | BatchStatus::ProofPending { .. }
                | BatchStatus::ProofReady { .. }
        );
        let at_least_da_complete = matches!(
            status,
            BatchStatus::DaComplete { .. }
                | BatchStatus::ProofPending { .. }
                | BatchStatus::ProofReady { .. }
        );
        let at_least_proof_pending = matches!(
            status,
            BatchStatus::ProofPending { .. } | BatchStatus::ProofReady { .. }
        );
        let at_least_proof_ready = matches!(status, BatchStatus::ProofReady { .. });

        // Update frontiers (only if not already found - we want the highest idx)
        if at_least_da_pending && !found_da_pending {
            state.da_pending = frontier;
            found_da_pending = true;
        }
        if at_least_da_complete && !found_da_complete {
            state.da_complete = frontier;
            found_da_complete = true;
        }
        if at_least_proof_pending && !found_proof_pending {
            state.proof_pending = frontier;
            found_proof_pending = true;
        }
        if at_least_proof_ready && !found_proof_ready {
            state.proof_ready = frontier;
            found_proof_ready = true;
        }

        // Early exit if all frontiers found
        if found_da_pending && found_da_complete && found_proof_pending && found_proof_ready {
            break;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use alpen_ee_common::InMemoryStorage;

    use super::*;
    use crate::batch_lifecycle::test_utils::{
        fill_storage, make_genesis_batch, TestBatchStatus::*,
    };

    fn genesis_state() -> BatchLifecycleState {
        let genesis = make_genesis_batch(0);
        BatchLifecycleState::new_at_genesis(genesis.id())
    }

    #[tokio::test]
    async fn test_recover_all_sealed() {
        let storage = InMemoryStorage::new_empty();
        fill_storage(&storage, &[Sealed, Sealed, Sealed]).await;

        let mut state = genesis_state();
        recover_from_storage(&mut state, &storage, 3).await.unwrap();

        // All frontiers remain at genesis
        assert_eq!(state.da_pending().idx(), 0);
        assert_eq!(state.da_complete().idx(), 0);
        assert_eq!(state.proof_pending().idx(), 0);
        assert_eq!(state.proof_ready().idx(), 0);
    }

    #[tokio::test]
    async fn test_recover_typical_pipeline() {
        let storage = InMemoryStorage::new_empty();
        fill_storage(
            &storage,
            &[
                ProofReady,
                ProofReady,
                ProofPending,
                DaComplete,
                DaPending,
                Sealed,
            ],
        )
        .await;

        let mut state = genesis_state();
        recover_from_storage(&mut state, &storage, 6).await.unwrap();

        assert_eq!(state.proof_ready().idx(), 2);
        assert_eq!(state.proof_pending().idx(), 3);
        assert_eq!(state.da_complete().idx(), 4);
        assert_eq!(state.da_pending().idx(), 5);
    }

    #[tokio::test]
    async fn test_recover_all_proof_ready() {
        let storage = InMemoryStorage::new_empty();
        fill_storage(&storage, &[ProofReady, ProofReady, ProofReady]).await;

        let mut state = genesis_state();
        recover_from_storage(&mut state, &storage, 3).await.unwrap();

        // All frontiers at idx=3 (highest batch)
        assert_eq!(state.da_pending().idx(), 3);
        assert_eq!(state.da_complete().idx(), 3);
        assert_eq!(state.proof_pending().idx(), 3);
        assert_eq!(state.proof_ready().idx(), 3);
    }

    #[tokio::test]
    async fn test_recover_partial_progress() {
        let storage = InMemoryStorage::new_empty();
        fill_storage(&storage, &[DaComplete, DaPending, Sealed]).await;

        let mut state = genesis_state();
        recover_from_storage(&mut state, &storage, 3).await.unwrap();

        assert_eq!(state.da_pending().idx(), 2);
        assert_eq!(state.da_complete().idx(), 1);
        assert_eq!(state.proof_pending().idx(), 0); // genesis
        assert_eq!(state.proof_ready().idx(), 0); // genesis
    }

    #[tokio::test]
    async fn test_recover_single_batch_proof_ready() {
        let storage = InMemoryStorage::new_empty();
        fill_storage(&storage, &[ProofReady]).await;

        let mut state = genesis_state();
        recover_from_storage(&mut state, &storage, 1).await.unwrap();

        // Single ProofReady batch satisfies all frontiers
        assert_eq!(state.da_pending().idx(), 1);
        assert_eq!(state.da_complete().idx(), 1);
        assert_eq!(state.proof_pending().idx(), 1);
        assert_eq!(state.proof_ready().idx(), 1);
    }
}
