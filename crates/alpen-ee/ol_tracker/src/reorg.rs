use alpen_ee_common::{
    chain_status_checked, EeAccountStateAtEpoch, OLChainStatus, OLClient, Storage,
};
use tracing::{debug, error, info, warn};

use crate::{
    ctx::OLTrackerCtx,
    error::{OLTrackerError, Result},
    state::{build_tracker_state, OLTrackerState},
};

/// Finds the last common epoch state between local storage and remote chain.
pub(crate) async fn find_fork_point<TStorage, TOLClient>(
    storage: &TStorage,
    ol_client: &TOLClient,
    genesis_epoch: u32,
    latest_confirmed_epoch: u32,
) -> Result<Option<EeAccountStateAtEpoch>>
where
    TStorage: Storage,
    TOLClient: OLClient,
{
    if genesis_epoch > latest_confirmed_epoch {
        warn!(
            %genesis_epoch,
            %latest_confirmed_epoch,
            "empty search range: genesis epoch is beyond latest confirmed epoch"
        );
        return Ok(None);
    }

    for current_epoch in (genesis_epoch..=latest_confirmed_epoch).rev() {
        debug!(%current_epoch, "checking epoch for fork point");

        // Fetch the epoch summary to get the terminal block ID
        let epoch_summary = ol_client.epoch_summary(current_epoch).await?;
        let terminal_blkid = epoch_summary.epoch().last_blkid();

        // Check if we have this epoch's terminal block in our storage
        if let Some(state) = storage.ee_account_state(terminal_blkid.into()).await? {
            info!(epoch = %current_epoch, "found fork point");
            return Ok(Some(state));
        }
    }

    Ok(None)
}

/// Rolls back storage to fork point and updates internal tracker state.
pub(crate) async fn rollback_to_fork_point<TStorage>(
    state: &mut OLTrackerState,
    storage: &TStorage,
    fork_state: &EeAccountStateAtEpoch,
    ol_status: &OLChainStatus,
) -> Result<()>
where
    TStorage: Storage,
{
    let epoch = fork_state.epoch_commitment().epoch();

    info!(%epoch, "rolling back to fork point");

    // Build next state first. If this fails, db rollback will not occur and this operation can be
    // re-triggered in the next cycle.
    let next_state = build_tracker_state(fork_state.clone(), ol_status, storage).await?;
    debug!(?next_state, "reorg: next tracker state");

    // Atomically rollback the db.
    // CRITICAL: This MUST be the last fallible operation during reorg handling before state
    // mutation.
    storage.rollback_ee_account_state(epoch).await?;
    *state = next_state;

    Ok(())
}

/// Handles chain reorganization by finding fork point and rolling back state.
pub(crate) async fn handle_reorg<TStorage, TOLClient>(
    state: &mut OLTrackerState,
    ctx: &OLTrackerCtx<TStorage, TOLClient>,
) -> Result<()>
where
    TStorage: Storage,
    TOLClient: OLClient,
{
    let genesis_epoch = ctx.params.genesis_ol_epoch();

    let ol_status = chain_status_checked(ctx.ol_client.as_ref()).await?;

    let fork_state = find_fork_point(
        ctx.storage.as_ref(),
        ctx.ol_client.as_ref(),
        genesis_epoch,
        ol_status.confirmed.epoch(),
    )
    .await?
    .ok_or_else(|| {
        error!(
            %genesis_epoch,
            "reorg: could not find ol fork epoch till ol genesis epoch"
        );
        OLTrackerError::NoForkPointFound {
            genesis_epoch: genesis_epoch.into(),
        }
    })?;

    warn!(
        epoch = %fork_state.epoch_commitment().epoch(),
        "reorg: found fork point; starting db rollback"
    );

    rollback_to_fork_point(state, ctx.storage.as_ref(), &fork_state, &ol_status).await?;

    ctx.notify_ol_status_update(state.get_ol_status());
    ctx.notify_consensus_update(state.get_consensus_heads());

    info!("reorg: reorg complete");

    Ok(())
}

#[cfg(test)]
mod tests {
    use alpen_ee_common::{MockOLClient, MockStorage, OLBlockOrEpoch, OLClientError, StorageError};
    use strata_identifiers::{Buf32, OLBlockId};

    use super::*;
    use crate::test_utils::*;

    mod find_fork_point_tests {

        use super::*;

        #[tokio::test]
        async fn test_finds_fork_at_epoch_2_with_divergence_at_3() {
            // Scenario: Chain diverges at epoch 3
            // Local storage:  [10, 11, 12, 13, 14] (epochs 0-4)
            // Remote chain:   [10, 11, 12, 99, 98, 99] (epochs 0-5)
            // Fork point: epoch 2 with terminal block ID 12
            let local_chain = create_epochs(&[10, 11, 12, 13, 14]);
            let remote_chain = create_epochs(&[10, 11, 12, 99, 98, 99]);

            let mut mock_storage = MockStorage::new();
            let mut mock_client = MockOLClient::new();

            setup_mock_client_with_chain(&mut mock_client, remote_chain);
            setup_mock_storage_with_chain(&mut mock_storage, local_chain);

            let result = find_fork_point(&mock_storage, &mock_client, 0, 5)
                .await
                .unwrap();

            assert!(result.is_some());
            let fork_state = result.unwrap();
            assert_eq!(fork_state.epoch_commitment().epoch(), 2);
            assert_eq!(fork_state.epoch_commitment().last_blkid().as_ref()[0], 12);
        }

        #[tokio::test]
        async fn test_local_behind_remote_no_divergence() {
            // Scenario: Local chain is behind remote but no divergence (subset case)
            // Local storage:  [100, 101, 102, 103] (epochs 0-3)
            // Remote chain:   [100, 101, 102, 103, 104, 105] (epochs 0-5)
            // Fork point: epoch 3 (last local epoch)
            let local_chain = create_epochs(&[100, 101, 102, 103]);
            let remote_chain = create_epochs(&[100, 101, 102, 103, 104, 105]);

            let mut mock_storage = MockStorage::new();
            let mut mock_client = MockOLClient::new();

            setup_mock_client_with_chain(&mut mock_client, remote_chain);
            setup_mock_storage_with_chain(&mut mock_storage, local_chain);

            let result = find_fork_point(&mock_storage, &mock_client, 0, 5)
                .await
                .unwrap();

            assert!(result.is_some());
            let fork_state = result.unwrap();
            assert_eq!(fork_state.epoch_commitment().epoch(), 3);
            assert_eq!(fork_state.epoch_commitment().last_blkid().as_ref()[0], 103);
        }

        #[tokio::test]
        async fn test_returns_none_when_no_fork_point_found() {
            // Scenario: Local storage is completely empty
            // Local storage:  [1, 2, 3, 4, 5] (epochs 0-4)
            // Remote chain:   [100, 101, 102, ...] (epochs 0-10)
            // No fork point found

            let local_chain = create_epochs(&[1, 2, 3, 4, 5]);
            let remote_chain =
                create_epochs(&[100, 101, 102, 103, 104, 105, 106, 107, 108, 109, 110]);

            let mut mock_storage = MockStorage::new();
            let mut mock_client = MockOLClient::new();

            setup_mock_storage_with_chain(&mut mock_storage, local_chain);
            setup_mock_client_with_chain(&mut mock_client, remote_chain);

            let result = find_fork_point(&mock_storage, &mock_client, 0, 10)
                .await
                .unwrap();

            assert!(result.is_none());
        }

        #[tokio::test]
        async fn test_respects_genesis_epoch_boundary() {
            // Scenario: Search only within specified range, storage has no matching epochs
            // Local storage:  [100, 101] (epochs 0,1)
            // Remote chain:   [100, 101, 102, 103, 104, 105] (epochs 0-5)
            // Genesis epoch: 2
            // Search range:   epochs 2-5
            // No fork point found (searches only within range, doesn't go beyond genesis)

            let local_chain = create_epochs(&[100, 101]);
            let remote_chain = create_epochs(&[100, 101, 102, 103, 104, 105]);

            let mut mock_storage = MockStorage::new();
            let mut mock_client = MockOLClient::new();

            setup_mock_storage_with_chain(&mut mock_storage, local_chain);
            setup_mock_client_with_chain(&mut mock_client, remote_chain);

            let result = find_fork_point(&mock_storage, &mock_client, 2, 5)
                .await
                .unwrap();

            assert!(result.is_none());
        }

        #[tokio::test]
        async fn test_propagates_client_error() {
            let mock_storage = MockStorage::new();
            let mut mock_client = MockOLClient::new();

            mock_client
                .expect_epoch_summary()
                .times(1)
                .returning(|_| Err(OLClientError::network("test error")));

            let result = find_fork_point(&mock_storage, &mock_client, 100, 110).await;

            assert!(matches!(result, Err(OLTrackerError::OLClient(_))));
        }

        #[tokio::test]
        async fn test_propagates_storage_error() {
            let remote_chain =
                create_epochs(&[100, 101, 102, 103, 104, 105, 106, 107, 108, 109, 110]);

            let mut mock_storage = MockStorage::new();
            let mut mock_client = MockOLClient::new();

            setup_mock_client_with_chain(&mut mock_client, remote_chain);

            mock_storage
                .expect_ee_account_state()
                .times(1)
                .returning(|_| Err(StorageError::database("test error")));

            let result = find_fork_point(&mock_storage, &mock_client, 0, 10).await;

            assert!(matches!(result, Err(OLTrackerError::Storage(_))));
        }

        #[tokio::test]
        async fn test_single_epoch_range() {
            // Scenario: Single epoch range (genesis_epoch == latest_confirmed_epoch)
            // Local storage:  [100] (epoch 0 with terminal block ID 100)
            // Remote chain:   [100] (epoch 0 with terminal block ID 100)
            // Fork point: epoch 0

            let local_chain = create_epochs(&[100]);
            let remote_chain = create_epochs(&[100]);

            let mut mock_storage = MockStorage::new();
            let mut mock_client = MockOLClient::new();

            setup_mock_client_with_chain(&mut mock_client, remote_chain);
            setup_mock_storage_with_chain(&mut mock_storage, local_chain);

            let result = find_fork_point(&mock_storage, &mock_client, 0, 0)
                .await
                .unwrap();

            assert!(result.is_some());
            assert_eq!(result.unwrap().epoch_commitment().epoch(), 0);
        }

        #[tokio::test]
        async fn test_empty_range_returns_none() {
            // Scenario: genesis_epoch > latest_confirmed_epoch (invalid/empty range)
            // This triggers the early return with warning log
            let mock_storage = MockStorage::new();
            let mock_client = MockOLClient::new();

            // No expectations needed - should return early

            let result = find_fork_point(&mock_storage, &mock_client, 100, 50)
                .await
                .unwrap();

            assert!(result.is_none());
        }
    }

    mod rollback_to_fork_point_tests {
        use super::*;

        #[tokio::test]
        async fn test_performs_rollback_and_builds_state() {
            // Scenario: Rollback to fork point
            // Fork point:     epoch 0 with terminal block ID 100
            // OL status:      confirmed=epoch 5, finalized=epoch 0
            // Expected:       DB rolled back to epoch 0, tracker state rebuilt

            let chain = create_epochs(&[100, 101, 102, 103, 104, 105]);
            let fork_state = chain[0].clone();

            let mut mock_storage = MockStorage::new();

            let ol_status = OLChainStatus {
                latest: make_block_commitment(60, 105),
                confirmed: make_epoch_commitment(5, 50, 105),
                finalized: make_epoch_commitment(0, 0, 100),
            };

            mock_storage
                .expect_rollback_ee_account_state()
                .times(1)
                .withf(|epoch| *epoch == 0)
                .returning(|_| Ok(()));

            setup_mock_storage_with_chain(&mut mock_storage, chain.clone());

            let mut state = OLTrackerState::new(chain[5].clone(), chain[0].clone());
            let result =
                rollback_to_fork_point(&mut state, &mock_storage, &fork_state, &ol_status).await;

            assert!(result.is_ok());
            assert_eq!(state.best_ee_state(), fork_state.ee_state());
        }
    }

    mod handle_reorg_tests {
        use std::sync::Arc;

        use alloy_primitives::B256;
        use alpen_ee_common::{ConsensusHeads, OLFinalizedStatus};
        use alpen_ee_config::AlpenEeParams;
        use strata_acct_types::{AccountId, Hash};
        use tokio::sync::watch;

        use super::*;

        fn make_test_params(genesis_epoch: u32) -> AlpenEeParams {
            let mut bytes = [0u8; 32];
            bytes[0] = genesis_epoch as u8;
            AlpenEeParams::new(
                AccountId::new([0; 32]),
                B256::ZERO,
                B256::ZERO,
                genesis_epoch,
                0,
                OLBlockId::from(Buf32::new(bytes)),
            )
        }

        fn make_test_ctx(
            storage: MockStorage,
            ol_client: MockOLClient,
            genesis_epoch: u32,
        ) -> OLTrackerCtx<MockStorage, MockOLClient> {
            let (ol_status_tx, _) = watch::channel(OLFinalizedStatus {
                ol_block: make_block_commitment(0, 0),
                last_ee_block: Hash::new([0; 32]),
            });
            let (consensus_tx, _) = watch::channel(ConsensusHeads {
                confirmed: Hash::new([0; 32]),
                finalized: Hash::new([0; 32]),
            });

            let params = make_test_params(genesis_epoch);

            OLTrackerCtx {
                storage: Arc::new(storage),
                ol_client: Arc::new(ol_client),
                params: Arc::new(params),
                ol_status_tx,
                consensus_tx,
                max_epochs_fetch: 10,
                poll_wait_ms: 100,
            }
        }

        #[tokio::test]
        async fn test_propagates_chain_status_error() {
            // Scenario: OL client error when fetching chain status
            // Expected:       Error propagated from OL client

            let mock_storage = MockStorage::new();
            let mut mock_client = MockOLClient::new();

            mock_client
                .expect_chain_status()
                .times(1)
                .returning(|| Err(OLClientError::network("network error")));

            let ctx = make_test_ctx(mock_storage, mock_client, 100);
            let mut state = OLTrackerState::new(
                make_state_at_epoch(105, 1050, 3, 3),
                make_state_at_epoch(100, 1000, 1, 1),
            );

            let result = handle_reorg(&mut state, &ctx).await;

            assert!(result.is_err());
        }

        #[tokio::test]
        async fn test_state_unchanged_when_build_tracker_state_fails() {
            // Scenario: Atomicity test - build_tracker_state fails before rollback
            // Genesis:        epoch 0
            // Local storage:  [100, 101, 102, 103, 104, 105, 106, 107] (epochs 0-7, but finalized
            // block read fails) Remote chain:   [100, 101, 102, 103, 104, 105, 106,
            // 107] (epochs 0-7) Fork point:     epoch 7 found
            // Failure:        build_tracker_state fails reading finalized block (id=100)
            // Expected:       Error returned, state unchanged, DB NOT rolled back

            let chain = create_epochs(&[100, 101, 102, 103, 104, 105, 106, 107]);

            let mut mock_storage = MockStorage::new();
            let mut mock_client = MockOLClient::new();

            mock_client.expect_chain_status().times(1).returning(|| {
                Ok(OLChainStatus {
                    latest: make_block_commitment(80, 110),
                    confirmed: make_epoch_commitment(5, 50, 105),
                    finalized: make_epoch_commitment(0, 0, 100),
                })
            });

            setup_mock_client_with_chain(&mut mock_client, chain.clone());

            // Custom storage mock that simulates failure when reading finalized block
            let chain_for_storage = chain.clone();
            mock_storage
                .expect_ee_account_state()
                .times(..)
                .returning(move |block_or_slot| match block_or_slot {
                    OLBlockOrEpoch::TerminalBlock(block_id) => {
                        let id_byte = block_id.as_ref()[0];
                        // Simulate failure when reading finalized block (which has id=100)
                        if id_byte == 100 {
                            return Err(StorageError::database(
                                "simulated storage read failure for finalized block",
                            ));
                        }
                        for state in &chain_for_storage {
                            if state.epoch_commitment().last_blkid().as_ref()[0] == id_byte {
                                return Ok(Some(state.clone()));
                            }
                        }
                        Ok(None)
                    }
                    OLBlockOrEpoch::Epoch(epoch) => {
                        let state = &chain_for_storage[epoch as usize];
                        Ok(Some(state.clone()))
                    }
                });

            // DB rollback should NOT be called because build_tracker_state fails first
            mock_storage.expect_rollback_ee_account_state().times(0);

            let ctx = make_test_ctx(mock_storage, mock_client, 0);
            let mut state = OLTrackerState::new(chain[5].clone(), chain[0].clone());

            let result = handle_reorg(&mut state, &ctx).await;

            // Reorg should fail
            assert!(result.is_err());
            assert!(result
                .unwrap_err()
                .to_string()
                .contains("simulated storage read failure"));

            // State should remain unchanged
            assert_eq!(state.best_ol_epoch().epoch(), 5);
            assert_eq!(state.best_ol_epoch().last_blkid().as_ref()[0], 105);
        }

        #[tokio::test]
        async fn test_state_unchanged_when_rollback_fails() {
            // Scenario: Atomicity test - DB rollback fails
            // Genesis:        epoch 0
            // Local storage:  [100, 101, 102, 103, 104] (epochs 0-4)
            // Remote chain:   [100, 101, 112, 113, 144] (epochs 0-4)
            // Fork point:     epoch 1 found
            // Success:        build_tracker_state succeeds
            // Failure:        DB rollback fails
            // Expected:       Error returned, state unchanged (critical atomicity guarantee)

            let local_chain = create_epochs(&[100, 101, 102, 103, 104]);
            let remote_chain = create_epochs(&[100, 101, 112, 113, 114]);

            let mut mock_storage = MockStorage::new();
            let mut mock_client = MockOLClient::new();

            mock_client.expect_chain_status().times(1).returning(|| {
                Ok(OLChainStatus {
                    latest: make_block_commitment(80, 118),
                    confirmed: make_epoch_commitment(4, 40, 114),
                    finalized: make_epoch_commitment(0, 0, 100),
                })
            });

            setup_mock_client_with_chain(&mut mock_client, remote_chain.clone());
            setup_mock_storage_with_chain(&mut mock_storage, local_chain.clone());

            // DB rollback fails
            mock_storage
                .expect_rollback_ee_account_state()
                .times(1)
                .returning(|_| Err(StorageError::database("simulated rollback failure")));

            let ctx = make_test_ctx(mock_storage, mock_client, 0);
            let mut state = OLTrackerState::new(local_chain[4].clone(), local_chain[0].clone());

            let result = handle_reorg(&mut state, &ctx).await;

            // Reorg should fail
            assert!(result.is_err());
            assert!(result
                .unwrap_err()
                .to_string()
                .contains("simulated rollback failure"));

            // State should remain unchanged - this is the critical atomicity guarantee
            assert_eq!(state.best_ol_epoch().epoch(), 4);
            assert_eq!(state.best_ol_epoch().last_blkid().as_ref()[0], 104);
        }
    }
}
