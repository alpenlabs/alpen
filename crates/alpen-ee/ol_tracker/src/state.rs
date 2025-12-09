use std::sync::Arc;

use alpen_ee_common::{ConsensusHeads, EeAccountStateAtEpoch, OLChainStatus, Storage};
use strata_ee_acct_types::EeAccountState;
use strata_identifiers::EpochCommitment;

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
    ol_chain_status: OLChainStatus,
    storage: Arc<TStorage>,
) -> Result<OLTrackerState>
where
    TStorage: Storage,
{
    let Some(best_state) = storage.best_ee_account_state().await? else {
        // nothing in storage, expected atleast genesis epoch info to be present
        return Err(OLTrackerError::MissingGenesisEpoch);
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

#[cfg(test)]
mod tests {
    use alpen_ee_common::{MockStorage, OLBlockOrEpoch, OLChainStatus, StorageError};

    use super::*;
    use crate::test_utils::*;

    mod effective_account_state_tests {
        use super::*;

        #[tokio::test]
        async fn test_returns_state_for_local_epoch_when_local_slot_is_lower() {
            // Scenario: Local epoch has lower slot than OL epoch
            // Local:    epoch 1 at slot 10 with terminal block ID 101
            // OL:       epoch 2 at slot 20 with terminal block ID 102
            // Expected: Returns state for local epoch (terminal block 101)

            let chain = create_epochs(&[100, 101, 102]);

            let mut mock_storage = MockStorage::new();
            setup_mock_storage_with_chain(&mut mock_storage, chain);

            let local = make_epoch_commitment(1, 10, 101);
            let ol = make_epoch_commitment(2, 20, 102);

            let result = effective_account_state(&local, &ol, &mock_storage)
                .await
                .unwrap();

            assert_eq!(result.epoch_commitment().epoch(), 1);
            assert_eq!(result.epoch_commitment().last_blkid().as_ref()[0], 101);
        }

        #[tokio::test]
        async fn test_returns_state_for_ol_epoch_when_ol_slot_is_lower() {
            // Scenario: OL epoch has lower slot than local epoch
            // Local:    epoch 2 at slot 20 with terminal block ID 102
            // OL:       epoch 1 at slot 10 with terminal block ID 101
            // Expected: Returns state for OL epoch (terminal block 101)

            let chain = create_epochs(&[100, 101, 102]);

            let mut mock_storage = MockStorage::new();
            setup_mock_storage_with_chain(&mut mock_storage, chain);

            let local = make_epoch_commitment(2, 20, 102);
            let ol = make_epoch_commitment(1, 10, 101);

            let result = effective_account_state(&local, &ol, &mock_storage)
                .await
                .unwrap();

            assert_eq!(result.epoch_commitment().epoch(), 1);
            assert_eq!(result.epoch_commitment().last_blkid().as_ref()[0], 101);
        }

        #[tokio::test]
        async fn test_returns_state_for_ol_epoch_when_slots_are_equal() {
            // Scenario: Local and OL epochs have equal slots
            // Local:    epoch 1 at slot 10 with terminal block ID 101
            // OL:       epoch 2 at slot 10 with terminal block ID 102
            // Expected: Returns state for OL epoch (takes ol when equal)

            let chain = create_epochs(&[100, 101, 102]);

            let mut mock_storage = MockStorage::new();
            setup_mock_storage_with_chain(&mut mock_storage, chain);

            let local = make_epoch_commitment(1, 10, 101);
            let ol = make_epoch_commitment(2, 10, 102);

            let result = effective_account_state(&local, &ol, &mock_storage)
                .await
                .unwrap();

            assert_eq!(result.epoch_commitment().epoch(), 2);
            assert_eq!(result.epoch_commitment().last_blkid().as_ref()[0], 102);
        }

        #[tokio::test]
        async fn test_returns_missing_block_error_when_epoch_not_found() {
            // Scenario: Storage doesn't have the requested epoch
            // Local:    epoch 1 at slot 10 with terminal block ID 101
            // OL:       epoch 2 at slot 20 with terminal block ID 102
            // Storage:  empty (no epochs stored)
            // Expected: MissingBlock error

            let mut mock_storage = MockStorage::new();

            mock_storage
                .expect_ee_account_state()
                .times(1)
                .returning(|_| Ok(None));

            let local = make_epoch_commitment(1, 10, 101);
            let ol = make_epoch_commitment(2, 20, 102);

            let result = effective_account_state(&local, &ol, &mock_storage).await;

            assert!(result.is_err());
            let error = result.unwrap_err();
            assert!(matches!(error, OLTrackerError::MissingBlock { .. }));
            assert!(error.to_string().contains("missing expected block"));
        }

        #[tokio::test]
        async fn test_propagates_storage_error() {
            // Scenario: Storage returns an error
            // Local:    epoch 1 at slot 10 with terminal block ID 101
            // OL:       epoch 2 at slot 20 with terminal block ID 102
            // Expected: Storage error propagated

            let mut mock_storage = MockStorage::new();

            mock_storage
                .expect_ee_account_state()
                .times(1)
                .returning(|_| Err(StorageError::database("database connection failed")));

            let local = make_epoch_commitment(1, 10, 101);
            let ol = make_epoch_commitment(2, 20, 102);

            let result = effective_account_state(&local, &ol, &mock_storage).await;

            assert!(result.is_err());
            let error = result.unwrap_err();
            assert!(matches!(error, OLTrackerError::Storage(_)));
            assert!(error.to_string().contains("database connection failed"));
        }
    }

    mod build_tracker_state_tests {
        use strata_acct_types::Hash;

        use super::*;

        #[tokio::test]
        async fn test_builds_state_successfully() {
            // Scenario: Build tracker state with valid chain
            // Local chain: [100, 101, 102, 103, 104, 105] (epochs 0-5)
            // Best state:  epoch 5 (terminal block 105)
            // OL status:   latest=epoch 5, confirmed=epoch 4, finalized=epoch 2
            // Expected:    State built with confirmed=epoch 4, finalized=epoch 2

            let chain = create_epochs(&[100, 101, 102, 103, 104, 105]);
            let best_state = chain[5].clone();

            let mut mock_storage = MockStorage::new();
            setup_mock_storage_with_chain(&mut mock_storage, chain);

            let ol_status = OLChainStatus {
                latest: make_block_commitment(50, 105),
                confirmed: make_epoch_commitment(4, 40, 104),
                finalized: make_epoch_commitment(2, 20, 102),
            };

            let result = build_tracker_state(best_state, &ol_status, &mock_storage)
                .await
                .unwrap();

            assert_eq!(result.best_ol_epoch().epoch(), 4);

            // Verify consensus heads were set correctly
            let consensus = result.get_consensus_heads();
            let mut expected_confirmed = [0u8; 32];
            expected_confirmed[0] = 104;
            let mut expected_finalized = [0u8; 32];
            expected_finalized[0] = 102;

            assert_eq!(consensus.confirmed, Hash::from(expected_confirmed));
            assert_eq!(consensus.finalized, Hash::from(expected_finalized));
        }

        #[tokio::test]
        async fn test_returns_build_state_failed_when_confirmed_missing() {
            // Scenario: Confirmed epoch is missing from storage
            // Local chain: empty
            // Best state:  epoch 5 (terminal block 105)
            // OL status:   confirmed=epoch 4 (not in storage)
            // Expected:    BuildStateFailed error for confirmed state

            let mut mock_storage = MockStorage::new();

            let best_state = make_state_at_epoch(5, 50, 105, 105);
            let ol_status = OLChainStatus {
                latest: make_block_commitment(50, 105),
                confirmed: make_epoch_commitment(4, 40, 104),
                finalized: make_epoch_commitment(2, 20, 102),
            };

            // Confirmed epoch is missing
            mock_storage
                .expect_ee_account_state()
                .times(1)
                .returning(|_| Ok(None));

            let result = build_tracker_state(best_state, &ol_status, &mock_storage).await;

            assert!(result.is_err());
            let error = result.unwrap_err();
            assert!(matches!(error, OLTrackerError::BuildStateFailed(_)));
            assert!(error.to_string().contains("confirmed state"));
        }

        #[tokio::test]
        async fn test_returns_build_state_failed_when_finalized_missing() {
            // Scenario: Finalized epoch is missing from storage
            // Local chain: [100, 101, 102, 103, 104, 105] (epochs 0-5)
            // Best state:  epoch 5 (terminal block 105)
            // OL status:   confirmed=epoch 4 (exists), finalized=epoch 2 (missing)
            // Expected:    BuildStateFailed error for finalized state

            let chain = create_epochs(&[100, 101, 102, 103, 104, 105]);
            let best_state = chain[5].clone();

            let mut mock_storage = MockStorage::new();

            let ol_status = OLChainStatus {
                latest: make_block_commitment(50, 105),
                confirmed: make_epoch_commitment(4, 40, 104),
                finalized: make_epoch_commitment(2, 20, 102),
            };

            // First call for confirmed succeeds, second call for finalized returns None
            let chain_for_mock = chain.clone();
            mock_storage
                .expect_ee_account_state()
                .times(2)
                .returning(move |block_or_slot| match block_or_slot {
                    OLBlockOrEpoch::TerminalBlock(id) if id.as_ref()[0] == 104 => {
                        Ok(Some(chain_for_mock[4].clone()))
                    }
                    _ => Ok(None), // finalized is missing
                });

            let result = build_tracker_state(best_state, &ol_status, &mock_storage).await;

            assert!(result.is_err());
            let error = result.unwrap_err();
            assert!(matches!(error, OLTrackerError::BuildStateFailed(_)));
            assert!(error.to_string().contains("finalized state"));
        }

        #[tokio::test]
        async fn test_propagates_storage_error_in_build() {
            // Scenario: Storage returns an error during state building
            // Best state:  epoch 5 (terminal block 105)
            // OL status:   confirmed=epoch 4, finalized=epoch 2
            // Expected:    Storage error propagated as BuildStateFailed

            let best_state = make_state_at_epoch(5, 50, 105, 105);
            let ol_status = OLChainStatus {
                latest: make_block_commitment(50, 105),
                confirmed: make_epoch_commitment(4, 40, 104),
                finalized: make_epoch_commitment(2, 20, 102),
            };

            let mut mock_storage = MockStorage::new();

            // Storage returns error
            mock_storage
                .expect_ee_account_state()
                .times(1)
                .returning(|_| Err(StorageError::database("disk error")));

            let result = build_tracker_state(best_state, &ol_status, &mock_storage).await;

            assert!(result.is_err());
            let error = result.unwrap_err();
            assert!(matches!(error, OLTrackerError::BuildStateFailed(_)));
            assert!(error.to_string().contains("confirmed state"));
            assert!(error.to_string().contains("disk error"));
        }
    }
}
