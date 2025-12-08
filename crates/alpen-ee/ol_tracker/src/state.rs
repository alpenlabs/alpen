use std::sync::Arc;

use alpen_ee_common::{ConsensusHeads, EeAccountStateAtEpoch, OLChainStatus, Storage};
use alpen_ee_config::AlpenEeConfig;
use strata_acct_types::BitcoinAmount;
use strata_ee_acct_types::EeAccountState;
use strata_identifiers::EpochCommitment;
use tracing::warn;

use crate::error::{OLTrackerError, Result};

/// Internal State of the OL tracker.
#[derive(Debug, Clone)]
pub struct OLTrackerState {
    confirmed: EeAccountStateAtEpoch,
    finalized: EeAccountStateAtEpoch,
}

#[cfg(test)]
impl OLTrackerState {
    pub fn new(confirmed: EeAccountStateAtEpoch, finalized: EeAccountStateAtEpoch) -> Self {
        Self {
            confirmed,
            finalized,
        }
    }
}

impl OLTrackerState {
    /// Returns the best EE account state.
    pub fn best_ee_state(&self) -> &EeAccountState {
        self.confirmed.ee_state()
    }

    /// Returns the best OL block commitment.
    pub fn best_ol_epoch(&self) -> &EpochCommitment {
        self.confirmed.epoch_commitment()
    }

    /// Returns the consensus heads derived from confirmed and finalized states.
    pub fn get_consensus_heads(&self) -> ConsensusHeads {
        ConsensusHeads {
            confirmed: self.confirmed.last_exec_blkid(),
            finalized: self.finalized.last_exec_blkid(),
        }
    }

    /// Returns the OL chain status based on local state.
    /// This tracks the tips of the local view of the OL chain, which is expected to be available in
    /// local database.
    pub fn get_ol_status(&self) -> OLChainStatus {
        OLChainStatus {
            latest: self.confirmed.epoch_commitment().to_block_commitment(),
            confirmed: *self.confirmed.epoch_commitment(),
            finalized: *self.finalized.epoch_commitment(),
        }
    }
}

/// Initialized [`OLTrackerState`] from storage
pub async fn init_ol_tracker_state<TStorage>(
    config: Arc<AlpenEeConfig>,
    ol_chain_status: OLChainStatus,
    storage: Arc<TStorage>,
) -> Result<OLTrackerState>
where
    TStorage: Storage,
{
    let Some(best_state) = storage.best_ee_account_state().await? else {
        // nothing in storage, so initialize using genesis config

        warn!("ee state not found; create using genesis config");
        let genesis_state = EeAccountState::new(
            *config.params().genesis_blockhash().as_ref(),
            BitcoinAmount::zero(),
            vec![],
            vec![],
        );
        let genesis_ol_epoch = EpochCommitment::new(
            config.params().genesis_ol_epoch(),
            config.params().genesis_ol_slot(),
            config.params().genesis_ol_blockid(),
        );
        // persist genesis state
        storage
            .store_ee_account_state(&genesis_ol_epoch, &genesis_state)
            .await?;

        let block_account_state = EeAccountStateAtEpoch::new(genesis_ol_epoch, genesis_state);

        return Ok(OLTrackerState {
            confirmed: block_account_state.clone(),
            finalized: block_account_state,
        });
    };

    build_tracker_state(best_state, &ol_chain_status, storage.as_ref()).await
}

pub(crate) async fn build_tracker_state(
    best_state: EeAccountStateAtEpoch,
    ol_chain_status: &OLChainStatus,
    storage: &impl Storage,
) -> Result<OLTrackerState> {
    // determine confirmed, finalized states
    let confirmed_state = effective_account_state(
        best_state.epoch_commitment(),
        ol_chain_status.confirmed(),
        storage,
    )
    .await
    .map_err(|e| OLTrackerError::BuildStateFailed(format!("confirmed state: {}", e)))?;

    let finalized_state = effective_account_state(
        best_state.epoch_commitment(),
        ol_chain_status.finalized(),
        storage,
    )
    .await
    .map_err(|e| OLTrackerError::BuildStateFailed(format!("finalized state: {}", e)))?;

    Ok(OLTrackerState {
        confirmed: confirmed_state,
        finalized: finalized_state,
    })
}

async fn effective_account_state(
    local: &EpochCommitment,
    ol: &EpochCommitment,
    storage: &impl Storage,
) -> Result<EeAccountStateAtEpoch> {
    let min_epoch = if local.last_slot() < ol.last_slot() {
        local
    } else {
        ol
    };

    storage
        .ee_account_state(min_epoch.last_blkid().into())
        .await?
        .ok_or_else(|| OLTrackerError::MissingBlock {
            block_id: min_epoch.to_string(),
        })
}

// #[cfg(test)]
// mod tests {
//     use alpen_ee_common::{MockStorage, OLBlockOrSlot, OLChainStatus, StorageError};
//     use strata_acct_types::Hash;
//     use strata_identifiers::Buf32;

//     use super::*;

//     /// Helper to create a block commitment for testing
//     fn make_block_commitment(slot: u64, id: u8) -> OLBlockCommitment {
//         let mut bytes = [0u8; 32];
//         bytes[0] = id;
//         OLBlockCommitment::new(slot, Buf32::new(bytes).into())
//     }

//     /// Helper to create a test state at block
//     fn make_state_at_block(slot: u64, block_id: u8, exec_blkid: u8) -> EeAccountStateAtBlock {
//         let block = make_block_commitment(slot, block_id);
//         let mut exec_bytes = [0u8; 32];
//         exec_bytes[0] = exec_blkid;
//         let ee_state = EeAccountState::new(exec_bytes, BitcoinAmount::zero(), vec![], vec![]);
//         EeAccountStateAtBlock::new(block, ee_state)
//     }

//     mod effective_account_state_tests {
//         use super::*;

//         #[tokio::test]
//         async fn test_returns_state_for_local_block_when_local_slot_is_lower() {
//             let mut mock_storage = MockStorage::new();

//             let local = make_block_commitment(100, 1);
//             let ol = make_block_commitment(105, 2);

//             let expected_state = make_state_at_block(100, 1, 1);

//             // Should query for local block (id=1) since local slot (100) < ol slot (105)
//             mock_storage
//                 .expect_ee_account_state()
//                 .times(1)
//                 .withf(|block_or_slot| {
//                     matches!(block_or_slot, OLBlockOrSlot::Block(id) if id.as_ref()[0] == 1)
//                 })
//                 .returning(move |_| Ok(Some(expected_state.clone())));

//             let result = effective_account_state(&local, &ol, &mock_storage)
//                 .await
//                 .unwrap();

//             assert_eq!(result.ol_slot(), 100);
//             assert_eq!(result.ol_block().blkid().as_ref()[0], 1);
//         }

//         #[tokio::test]
//         async fn test_returns_state_for_ol_block_when_ol_slot_is_lower() {
//             let mut mock_storage = MockStorage::new();

//             let local = make_block_commitment(105, 1);
//             let ol = make_block_commitment(100, 2);

//             let expected_state = make_state_at_block(100, 2, 2);

//             // Should query for ol block (id=2) since ol slot (100) < local slot (105)
//             mock_storage
//                 .expect_ee_account_state()
//                 .times(1)
//                 .withf(|block_or_slot| {
//                     matches!(block_or_slot, OLBlockOrSlot::Block(id) if id.as_ref()[0] == 2)
//                 })
//                 .returning(move |_| Ok(Some(expected_state.clone())));

//             let result = effective_account_state(&local, &ol, &mock_storage)
//                 .await
//                 .unwrap();

//             assert_eq!(result.ol_slot(), 100);
//             assert_eq!(result.ol_block().blkid().as_ref()[0], 2);
//         }

//         #[tokio::test]
//         async fn test_returns_state_for_ol_block_when_slots_are_equal() {
//             let mut mock_storage = MockStorage::new();

//             let local = make_block_commitment(100, 1);
//             let ol = make_block_commitment(100, 2);

//             let expected_state = make_state_at_block(100, 2, 2);

//             // Should query for ol block (id=2) since slots are equal (takes ol)
//             mock_storage
//                 .expect_ee_account_state()
//                 .times(1)
//                 .withf(|block_or_slot| {
//                     matches!(block_or_slot, OLBlockOrSlot::Block(id) if id.as_ref()[0] == 2)
//                 })
//                 .returning(move |_| Ok(Some(expected_state.clone())));

//             let result = effective_account_state(&local, &ol, &mock_storage)
//                 .await
//                 .unwrap();

//             assert_eq!(result.ol_slot(), 100);
//             assert_eq!(result.ol_block().blkid().as_ref()[0], 2);
//         }

//         #[tokio::test]
//         async fn test_returns_missing_block_error_when_block_not_found() {
//             let mut mock_storage = MockStorage::new();

//             let local = make_block_commitment(100, 1);
//             let ol = make_block_commitment(105, 2);

//             // Storage returns None - block doesn't exist
//             mock_storage
//                 .expect_ee_account_state()
//                 .times(1)
//                 .returning(|_| Ok(None));

//             let result = effective_account_state(&local, &ol, &mock_storage).await;

//             assert!(result.is_err());
//             let error = result.unwrap_err();
//             assert!(matches!(error, OLTrackerError::MissingBlock { .. }));
//             assert!(error.to_string().contains("missing expected block"));
//         }

//         #[tokio::test]
//         async fn test_propagates_storage_error() {
//             let mut mock_storage = MockStorage::new();

//             let local = make_block_commitment(100, 1);
//             let ol = make_block_commitment(105, 2);

//             // Storage returns error
//             mock_storage
//                 .expect_ee_account_state()
//                 .times(1)
//                 .returning(|_| Err(StorageError::database("database connection failed")));

//             let result = effective_account_state(&local, &ol, &mock_storage).await;

//             assert!(result.is_err());
//             let error = result.unwrap_err();
//             assert!(matches!(error, OLTrackerError::Storage(_)));
//             assert!(error.to_string().contains("database connection failed"));
//         }

//         #[tokio::test]
//         async fn test_always_queries_min_slot_block() {
//             // This test verifies the min slot logic explicitly
//             let mut mock_storage = MockStorage::new();

//             // Test case 1: local=50, ol=100 -> should query id=1 (local)
//             let local1 = make_block_commitment(50, 1);
//             let ol1 = make_block_commitment(100, 2);

//             mock_storage
//                 .expect_ee_account_state()
//                 .times(1)
//                 .withf(|block_or_slot| {
//                     matches!(block_or_slot, OLBlockOrSlot::Block(id) if id.as_ref()[0] == 1)
//                 })
//                 .returning(|_| Ok(Some(make_state_at_block(50, 1, 1))));

//             let result1 = effective_account_state(&local1, &ol1, &mock_storage)
//                 .await
//                 .unwrap();
//             assert_eq!(result1.ol_slot(), 50);

//             // Test case 2: local=100, ol=50 -> should query id=2 (ol)
//             let local2 = make_block_commitment(100, 1);
//             let ol2 = make_block_commitment(50, 2);

//             mock_storage
//                 .expect_ee_account_state()
//                 .times(1)
//                 .withf(|block_or_slot| {
//                     matches!(block_or_slot, OLBlockOrSlot::Block(id) if id.as_ref()[0] == 2)
//                 })
//                 .returning(|_| Ok(Some(make_state_at_block(50, 2, 2))));

//             let result2 = effective_account_state(&local2, &ol2, &mock_storage)
//                 .await
//                 .unwrap();
//             assert_eq!(result2.ol_slot(), 50);
//         }
//     }

//     mod build_tracker_state_tests {
//         use super::*;

//         #[tokio::test]
//         async fn test_builds_state_successfully() {
//             let mut mock_storage = MockStorage::new();

//             let best_state = make_state_at_block(110, 1, 1);
//             let ol_status = OLChainStatus {
//                 latest: make_block_commitment(110, 1),
//                 confirmed: make_block_commitment(105, 2),
//                 finalized: make_block_commitment(100, 3),
//             };

//             // Mock responses for confirmed and finalized state queries
//             mock_storage
//                 .expect_ee_account_state()
//                 .times(2)
//                 .returning(|block_or_slot| match block_or_slot {
//                     OLBlockOrSlot::Block(id) if id.as_ref()[0] == 2 => {
//                         Ok(Some(make_state_at_block(105, 2, 2)))
//                     }
//                     OLBlockOrSlot::Block(id) if id.as_ref()[0] == 3 => {
//                         Ok(Some(make_state_at_block(100, 3, 3)))
//                     }
//                     _ => Ok(None),
//                 });

//             let result = build_tracker_state(best_state, &ol_status, &mock_storage)
//                 .await
//                 .unwrap();

//             assert_eq!(result.best_ol_block().slot(), 110);

//             // Verify consensus heads were set correctly
//             let consensus = result.get_consensus_heads();
//             let mut expected_confirmed = [0u8; 32];
//             expected_confirmed[0] = 2;
//             let mut expected_finalized = [0u8; 32];
//             expected_finalized[0] = 3;

//             assert_eq!(consensus.confirmed, Hash::from(expected_confirmed));
//             assert_eq!(consensus.finalized, Hash::from(expected_finalized));
//         }

//         #[tokio::test]
//         async fn test_returns_build_state_failed_when_confirmed_missing() {
//             let mut mock_storage = MockStorage::new();

//             let best_state = make_state_at_block(110, 1, 1);
//             let ol_status = OLChainStatus {
//                 latest: make_block_commitment(110, 1),
//                 confirmed: make_block_commitment(105, 2),
//                 finalized: make_block_commitment(100, 3),
//             };

//             // Confirmed block is missing
//             mock_storage
//                 .expect_ee_account_state()
//                 .times(1)
//                 .returning(|_| Ok(None));

//             let result = build_tracker_state(best_state, &ol_status, &mock_storage).await;

//             assert!(result.is_err());
//             let error = result.unwrap_err();
//             assert!(matches!(error, OLTrackerError::BuildStateFailed(_)));
//             assert!(error.to_string().contains("confirmed state"));
//         }

//         #[tokio::test]
//         async fn test_returns_build_state_failed_when_finalized_missing() {
//             let mut mock_storage = MockStorage::new();

//             let best_state = make_state_at_block(110, 1, 1);
//             let ol_status = OLChainStatus {
//                 latest: make_block_commitment(110, 1),
//                 confirmed: make_block_commitment(105, 2),
//                 finalized: make_block_commitment(100, 3),
//             };

//             // First call for confirmed succeeds, second call for finalized fails
//             mock_storage
//                 .expect_ee_account_state()
//                 .times(2)
//                 .returning(|block_or_slot| match block_or_slot {
//                     OLBlockOrSlot::Block(id) if id.as_ref()[0] == 2 => {
//                         Ok(Some(make_state_at_block(105, 2, 2)))
//                     }
//                     _ => Ok(None), // finalized is missing
//                 });

//             let result = build_tracker_state(best_state, &ol_status, &mock_storage).await;

//             assert!(result.is_err());
//             let error = result.unwrap_err();
//             assert!(matches!(error, OLTrackerError::BuildStateFailed(_)));
//             assert!(error.to_string().contains("finalized state"));
//         }

//         #[tokio::test]
//         async fn test_propagates_storage_error_in_build() {
//             let mut mock_storage = MockStorage::new();

//             let best_state = make_state_at_block(110, 1, 1);
//             let ol_status = OLChainStatus {
//                 latest: make_block_commitment(110, 1),
//                 confirmed: make_block_commitment(105, 2),
//                 finalized: make_block_commitment(100, 3),
//             };

//             // Storage returns error
//             mock_storage
//                 .expect_ee_account_state()
//                 .times(1)
//                 .returning(|_| Err(StorageError::database("disk error")));

//             let result = build_tracker_state(best_state, &ol_status, &mock_storage).await;

//             assert!(result.is_err());
//             let error = result.unwrap_err();
//             assert!(matches!(error, OLTrackerError::BuildStateFailed(_)));
//             assert!(error.to_string().contains("confirmed state"));
//             assert!(error.to_string().contains("disk error"));
//         }
//     }
