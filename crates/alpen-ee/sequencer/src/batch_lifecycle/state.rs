//! Batch lifecycle state management.

use alpen_ee_common::{require_latest_batch, BatchStatus, BatchStorage, StorageError};

/// State for tracking batch lifecycle progress.
///
/// The lifecycle manager processes batches sequentially through their lifecycle states.
/// This struct tracks four independent frontiers, one for each state transition.
/// Each frontier points to the **next batch to process** for that transition.
///
/// Batch Lifecycle States:
/// Sealed → DaPending → DaComplete → ProofPending → ProofReady
///
/// 4 Frontiers (each points to the next batch to process):
/// 1. da_post_frontier        - next batch to post DA for (Sealed → DaPending)
/// 2. da_confirm_frontier     - next batch to confirm DA for (DaPending → DaComplete)
/// 3. proof_request_frontier  - next batch to request proof for (DaComplete → ProofPending)
/// 4. proof_complete_frontier - next batch to complete proof for (ProofPending → ProofReady)
///
/// Invariant: `proof_complete <= proof_request <= da_confirm <= da_post`
///
/// Initialize using [`init_lifecycle_state`].
#[derive(Debug)]
pub struct BatchLifecycleState {
    /// Next batch to post DA for (Sealed → DaPending).
    da_post_frontier: u64,

    /// Next batch to confirm DA for (DaPending → DaComplete).
    da_confirm_frontier: u64,

    /// Next batch to request proof for (DaComplete → ProofPending).
    proof_request_frontier: u64,

    /// Next batch to complete proof for (ProofPending → ProofReady).
    proof_complete_frontier: u64,
}

impl BatchLifecycleState {
    /// Get the DA post frontier index.
    pub(crate) fn da_post_frontier(&self) -> u64 {
        self.da_post_frontier
    }

    /// Get the DA confirm frontier index.
    pub(crate) fn da_confirm_frontier(&self) -> u64 {
        self.da_confirm_frontier
    }

    /// Get the proof request frontier index.
    pub(crate) fn proof_request_frontier(&self) -> u64 {
        self.proof_request_frontier
    }

    /// Get the proof complete frontier index.
    pub(crate) fn proof_complete_frontier(&self) -> u64 {
        self.proof_complete_frontier
    }

    /// Advance DA post frontier.
    pub(crate) fn advance_da_post_frontier(&mut self) {
        self.da_post_frontier += 1;
    }

    /// Advance DA confirm frontier.
    pub(crate) fn advance_da_confirm_frontier(&mut self) {
        self.da_confirm_frontier += 1;
    }

    /// Advance proof request frontier.
    pub(crate) fn advance_proof_request_frontier(&mut self) {
        self.proof_request_frontier += 1;
    }

    /// Advance proof complete frontier.
    pub(crate) fn advance_proof_complete_frontier(&mut self) {
        self.proof_complete_frontier += 1;
    }

    /// Reset all frontiers to start fresh from genesis.
    pub(crate) fn reset_frontiers(&mut self) {
        self.da_post_frontier = 1; // Start at batch 1 (after genesis)
        self.da_confirm_frontier = 1;
        self.proof_request_frontier = 1;
        self.proof_complete_frontier = 1;
    }

    /// Create a new state for testing purposes.
    ///
    /// This constructor allows tests to create specific state configurations
    /// without going through storage initialization.
    #[cfg(test)]
    pub(crate) fn new_for_testing(
        da_post_frontier: u64,
        da_confirm_frontier: u64,
        proof_request_frontier: u64,
        proof_complete_frontier: u64,
    ) -> Self {
        Self {
            da_post_frontier,
            da_confirm_frontier,
            proof_request_frontier,
            proof_complete_frontier,
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

    // Find the frontier positions by scanning from batch 1
    let mut state = BatchLifecycleState {
        da_post_frontier: 1, // Start at batch 1 (after genesis)
        da_confirm_frontier: 1,
        proof_request_frontier: 1,
        proof_complete_frontier: 1,
    };

    // Scan batches to find where we are in the pipeline
    recover_from_storage(&mut state, storage, latest_idx).await?;

    Ok(state)
}

/// Recover state by scanning storage for batches in intermediate states.
///
/// Frontiers point to the **next batch to process**. We scan to find the first
/// incomplete batch and set frontiers accordingly.
pub(crate) async fn recover_from_storage(
    state: &mut BatchLifecycleState,
    storage: &impl BatchStorage,
    latest_idx: u64,
) -> Result<(), StorageError> {
    // Scan from idx 1 (skip genesis) to latest
    for idx in 1..=latest_idx {
        let Some((_batch, status)) = storage.get_batch_by_idx(idx).await? else {
            break;
        };

        match status {
            BatchStatus::Sealed => {
                // This batch needs DA posting - all frontiers point here
                state.da_post_frontier = idx;
                state.da_confirm_frontier = idx;
                state.proof_request_frontier = idx;
                state.proof_complete_frontier = idx;
                break;
            }
            BatchStatus::DaPending => {
                // DA posted, need confirmation
                state.da_post_frontier = idx + 1; // Move past this batch
                state.da_confirm_frontier = idx; // Confirming this batch
                state.proof_request_frontier = idx;
                state.proof_complete_frontier = idx;
                break;
            }
            BatchStatus::DaComplete { .. } => {
                // DA confirmed, need proof request
                state.da_post_frontier = idx + 1;
                state.da_confirm_frontier = idx + 1; // Move past
                state.proof_request_frontier = idx; // Request proof for this batch
                state.proof_complete_frontier = idx;
                break;
            }
            BatchStatus::ProofPending { .. } => {
                // Proof requested, need completion
                state.da_post_frontier = idx + 1;
                state.da_confirm_frontier = idx + 1;
                state.proof_request_frontier = idx + 1; // Move past
                state.proof_complete_frontier = idx; // Complete proof for this batch
                break;
            }
            BatchStatus::ProofReady { .. } => {
                // This batch is complete, advance all frontiers past it
                state.da_post_frontier = idx + 1;
                state.da_confirm_frontier = idx + 1;
                state.proof_request_frontier = idx + 1;
                state.proof_complete_frontier = idx + 1;
                // Continue scanning
            }
            BatchStatus::Genesis => unreachable!(),
        }
    }

    Ok(())
}

// #[cfg(test)]
// mod tests {
//     use alpen_ee_common::InMemoryStorage;

//     use super::*;
//     use crate::batch_lifecycle::test_utils::*;

//     // ========================================================================
//     // A. State Initialization
//     // ========================================================================

//     #[tokio::test]
//     async fn test_init_lifecycle_state_only_genesis() {
//         let storage = InMemoryStorage::new();

//         // Genesis batch at idx 0
//         let genesis = make_genesis_batch(0);
//         storage.save_genesis_batch(genesis.clone()).await.unwrap();

//         // Update genesis to ProofReady status
//         storage
//             .update_batch_status(
//                 genesis.id(),
//                 BatchStatus::ProofReady {
//                     da: vec![],
//                     proof: test_proof_id(0),
//                 },
//             )
//             .await
//             .unwrap();

//         let state = init_lifecycle_state(&storage).await.unwrap();

//         // Should start at batch 1 (after genesis)
//         assert_eq!(state.da_post_frontier(), 1);
//         assert_eq!(state.da_confirm_frontier(), 1);
//         assert_eq!(state.proof_request_frontier(), 1);
//         assert_eq!(state.proof_complete_frontier(), 1);
//     }

//     #[tokio::test]
//     async fn test_recover_finds_sealed_batch() {
//         let mut state = BatchLifecycleState::new_for_testing(1, 1, 1, 1);
//         let storage = InMemoryStorage::new();

//         // Store genesis
//         let genesis = make_genesis_batch(0);
//         storage.save_genesis_batch(genesis).await.unwrap();

//         // Store batches 1 and 2 as ProofReady
//         for idx in 1..=2 {
//             let batch = make_batch(idx, (idx - 1) as u8, idx as u8);
//             storage.save_next_batch(batch.clone()).await.unwrap();
//             storage
//                 .update_batch_status(
//                     batch.id(),
//                     BatchStatus::ProofReady {
//                         da: vec![make_da_ref(1, idx as u8)],
//                         proof: test_proof_id(idx as u8),
//                     },
//                 )
//                 .await
//                 .unwrap();
//         }

//         // Store batch 3 as Sealed
//         let batch3 = make_batch(3, 2, 3);
//         storage.save_next_batch(batch3).await.unwrap();

//         recover_from_storage(&mut state, &storage, 5).await.unwrap();

//         // All frontiers should stop at Sealed batch
//         assert_eq!(state.da_post_frontier(), 3);
//         assert_eq!(state.da_confirm_frontier(), 3);
//         assert_eq!(state.proof_request_frontier(), 3);
//         assert_eq!(state.proof_complete_frontier(), 3);
//     }

//     #[tokio::test]
//     async fn test_recover_finds_da_pending() {
//         let mut state = BatchLifecycleState::new_for_testing(1, 1, 1, 1);
//         let storage = InMemoryStorage::new();

//         // Store genesis
//         let genesis = make_genesis_batch(0);
//         storage.save_genesis_batch(genesis).await.unwrap();

//         // Store batches 1 and 2 as ProofReady
//         for idx in 1..=2 {
//             let batch = make_batch(idx, (idx - 1) as u8, idx as u8);
//             storage.save_next_batch(batch.clone()).await.unwrap();
//             storage
//                 .update_batch_status(
//                     batch.id(),
//                     BatchStatus::ProofReady {
//                         da: vec![make_da_ref(1, idx as u8)],
//                         proof: test_proof_id(idx as u8),
//                     },
//                 )
//                 .await
//                 .unwrap();
//         }

//         // Store batch 3 as DaPending
//         let batch3 = make_batch(3, 2, 3);
//         storage.save_next_batch(batch3.clone()).await.unwrap();
//         storage
//             .update_batch_status(batch3.id(), BatchStatus::DaPending)
//             .await
//             .unwrap();

//         recover_from_storage(&mut state, &storage, 5).await.unwrap();

//         // DA posted but not confirmed
//         assert_eq!(state.da_post_frontier(), 4); // Advance past
//         assert_eq!(state.da_confirm_frontier(), 3); // Waiting for confirmation
//         assert_eq!(state.proof_request_frontier(), 3);
//         assert_eq!(state.proof_complete_frontier(), 3);
//     }

//     #[tokio::test]
//     async fn test_recover_finds_proof_pending() {
//         let mut state = BatchLifecycleState::new_for_testing(1, 1, 1, 1);
//         let storage = InMemoryStorage::new();

//         // Store genesis
//         let genesis = make_genesis_batch(0);
//         storage.save_genesis_batch(genesis).await.unwrap();

//         // Store batches 1 and 2 as ProofReady
//         for idx in 1..=2 {
//             let batch = make_batch(idx, (idx - 1) as u8, idx as u8);
//             storage.save_next_batch(batch.clone()).await.unwrap();
//             storage
//                 .update_batch_status(
//                     batch.id(),
//                     BatchStatus::ProofReady {
//                         da: vec![make_da_ref(1, idx as u8)],
//                         proof: test_proof_id(idx as u8),
//                     },
//                 )
//                 .await
//                 .unwrap();
//         }

//         // Store batch 3 as ProofPending
//         let batch3 = make_batch(3, 2, 3);
//         storage.save_next_batch(batch3.clone()).await.unwrap();
//         storage
//             .update_batch_status(
//                 batch3.id(),
//                 BatchStatus::ProofPending {
//                     da: vec![make_da_ref(1, 3)],
//                 },
//             )
//             .await
//             .unwrap();

//         recover_from_storage(&mut state, &storage, 5).await.unwrap();

//         // Proof requested but not complete
//         assert_eq!(state.da_post_frontier(), 4);
//         assert_eq!(state.da_confirm_frontier(), 4);
//         assert_eq!(state.proof_request_frontier(), 4); // Advance past
//         assert_eq!(state.proof_complete_frontier(), 3); // Waiting for proof
//     }
// }
