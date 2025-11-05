use tracing::{debug, error, info, warn};

use super::{
    ctx::OlTrackerCtx,
    error::{OlTrackerError, Result},
    state::{build_tracker_state, OlTrackerState},
};
use crate::traits::{
    ol_client::{
        block_commitments_in_range_checked, chain_status_checked, OlChainStatus, OlClient,
    },
    storage::{EeAccountStateAtBlock, Storage},
};

/// Finds the last common block state between local storage and remote chain.
pub(super) async fn find_fork_point<TStorage, TOlClient>(
    storage: &TStorage,
    ol_client: &TOlClient,
    genesis_slot: u64,
    latest_slot: u64,
    fetch_size: u64,
) -> Result<Option<EeAccountStateAtBlock>>
where
    TStorage: Storage,
    TOlClient: OlClient,
{
    let mut max_slot = latest_slot;

    while max_slot >= genesis_slot {
        let min_slot = max_slot.saturating_sub(fetch_size).max(genesis_slot);

        debug!(min_slot, max_slot, "checking slot range for fork point");

        let blocks = block_commitments_in_range_checked(ol_client, min_slot, max_slot).await?;

        for block in blocks.iter().rev() {
            if let Some(state) = storage.ee_account_state(block.blkid().into()).await? {
                info!(slot = state.ol_slot(), "found fork point");
                return Ok(Some(state));
            }
        }

        if min_slot == genesis_slot {
            break;
        }
        max_slot = min_slot.saturating_sub(1);
    }

    Ok(None)
}

/// Rolls back storage to fork point and updates internal tracker state.
pub(super) async fn rollback_to_fork_point<TStorage>(
    state: &mut OlTrackerState,
    storage: &TStorage,
    fork_state: &EeAccountStateAtBlock,
    ol_status: &OlChainStatus,
) -> Result<()>
where
    TStorage: Storage,
{
    let slot = fork_state.ol_slot();

    info!(slot, "rolling back to fork point");

    // Build next state first. If this fails, db rollback will not occur and this operation can be
    // re-triggered in the next cycle.
    let next_state = build_tracker_state(fork_state.clone(), ol_status, storage).await?;
    debug!(?next_state, "reorg: next tracker state");

    // Atomically rollback the db.
    // CRITICAL: This MUST be the last fallible operation during reorg handling before state
    // mutation.
    storage.rollback_ee_account_state(slot).await?;
    *state = next_state;

    Ok(())
}

/// Handles chain reorganization by finding fork point and rolling back state.
pub(super) async fn handle_reorg<TStorage, TOlClient>(
    state: &mut OlTrackerState,
    ctx: &OlTrackerCtx<TStorage, TOlClient>,
) -> Result<()>
where
    TStorage: Storage,
    TOlClient: OlClient,
{
    let genesis_slot = ctx.params.genesis_ol_slot();

    let ol_status = chain_status_checked(ctx.ol_client.as_ref()).await?;

    let fork_state = find_fork_point(
        ctx.storage.as_ref(),
        ctx.ol_client.as_ref(),
        genesis_slot,
        ol_status.latest().slot(),
        ctx.reorg_fetch_size,
    )
    .await?
    .ok_or_else(|| {
        error!(
            genesis_slot,
            "reorg: could not find ol fork block till ol genesis slot"
        );
        OlTrackerError::NoForkPointFound { genesis_slot }
    })?;

    warn!(
        slot = fork_state.ol_slot(),
        "reorg: found fork point; starting db rollback"
    );

    rollback_to_fork_point(state, ctx.storage.as_ref(), &fork_state, &ol_status).await?;

    ctx.notify_state_update(state.best_ee_state());
    ctx.notify_consensus_update(state.get_consensus_heads());

    info!("reorg: reorg complete");

    Ok(())
}

#[cfg(test)]
mod tests {
    use strata_acct_types::BitcoinAmount;
    use strata_ee_acct_types::EeAccountState;
    use strata_identifiers::{Buf32, OLBlockCommitment};

    use super::*;
    use crate::traits::{
        ol_client::MockOlClient,
        storage::{MockStorage, OLBlockOrSlot},
    };

    fn make_block_commitment(slot: u64, id: u8) -> OLBlockCommitment {
        let mut bytes = [0u8; 32];
        bytes[0] = id;
        OLBlockCommitment::new(slot, Buf32::new(bytes).into())
    }

    fn make_ee_state(last_exec_blkid: [u8; 32]) -> EeAccountState {
        EeAccountState::new(last_exec_blkid, BitcoinAmount::zero(), vec![], vec![])
    }

    fn make_state_at_block(slot: u64, block_id: u8, state_id: u8) -> EeAccountStateAtBlock {
        let block = make_block_commitment(slot, block_id);
        let mut state_bytes = [0u8; 32];
        state_bytes[0] = state_id;
        let state = make_ee_state(state_bytes);
        EeAccountStateAtBlock::new(block, state)
    }

    mod find_fork_point_tests {
        use super::*;

        #[tokio::test]
        async fn test_finds_fork_point_in_first_batch() {
            let mut mock_storage = MockStorage::new();
            let mut mock_client = MockOlClient::new();

            mock_client
                .expect_block_commitments_in_range()
                .returning(|start, end| {
                    Ok((start..=end)
                        .map(|slot| make_block_commitment(slot, slot as u8))
                        .collect())
                });

            mock_storage
                .expect_ee_account_state()
                .returning(|block_or_slot| {
                    if let OLBlockOrSlot::Block(block_id) = block_or_slot {
                        if block_id.as_ref()[0] >= 105 {
                            return Ok(None);
                        }
                        let slot = block_id.as_ref()[0] as u64;
                        Ok(Some(make_state_at_block(slot, slot as u8, slot as u8)))
                    } else {
                        Ok(None)
                    }
                });

            let result = find_fork_point(&mock_storage, &mock_client, 100, 110, 50)
                .await
                .unwrap();

            assert!(result.is_some());
            let fork_state = result.unwrap();
            assert_eq!(fork_state.ol_slot(), 104);
        }

        #[tokio::test]
        async fn test_searches_multiple_batches() {
            let mut mock_storage = MockStorage::new();
            let mut mock_client = MockOlClient::new();

            mock_client
                .expect_block_commitments_in_range()
                .returning(|start, end| {
                    Ok((start..=end)
                        .map(|slot| make_block_commitment(slot, slot as u8))
                        .collect())
                });

            mock_storage
                .expect_ee_account_state()
                .returning(|block_or_slot| {
                    if let OLBlockOrSlot::Block(block_id) = block_or_slot {
                        if block_id.as_ref()[0] >= 86 {
                            return Ok(None);
                        }
                        let slot = block_id.as_ref()[0] as u64;
                        Ok(Some(make_state_at_block(slot, slot as u8, slot as u8)))
                    } else {
                        Ok(None)
                    }
                });

            let result = find_fork_point(&mock_storage, &mock_client, 80, 100, 11)
                .await
                .unwrap();

            assert!(result.is_some());
            assert_eq!(result.unwrap().ol_slot(), 85);
        }

        #[tokio::test]
        async fn test_returns_none_when_no_fork_point_found() {
            let mut mock_storage = MockStorage::new();
            let mut mock_client = MockOlClient::new();

            mock_client
                .expect_block_commitments_in_range()
                .times(1)
                .withf(|start, end| *start == 100 && *end == 110)
                .returning(|start, end| {
                    Ok((start..=end)
                        .map(|slot| make_block_commitment(slot, slot as u8))
                        .collect())
                });

            mock_storage
                .expect_ee_account_state()
                .returning(|_| Ok(None));

            let result = find_fork_point(&mock_storage, &mock_client, 100, 110, 50)
                .await
                .unwrap();

            assert!(result.is_none());
        }

        #[tokio::test]
        async fn test_respects_genesis_slot_boundary() {
            let mut mock_storage = MockStorage::new();
            let mut mock_client = MockOlClient::new();

            mock_client
                .expect_block_commitments_in_range()
                .times(1)
                .withf(|start, end| *start == 100 && *end == 105)
                .returning(|start, end| {
                    Ok((start..=end)
                        .map(|slot| make_block_commitment(slot, slot as u8))
                        .collect())
                });

            mock_storage
                .expect_ee_account_state()
                .returning(|_| Ok(None));

            let result = find_fork_point(&mock_storage, &mock_client, 100, 105, 50)
                .await
                .unwrap();

            assert!(result.is_none());
        }

        #[tokio::test]
        async fn test_handles_small_fetch_size() {
            let mut mock_storage = MockStorage::new();
            let mut mock_client = MockOlClient::new();

            mock_client
                .expect_block_commitments_in_range()
                .returning(|start, end| {
                    Ok((start..=end)
                        .map(|slot| make_block_commitment(slot, slot as u8))
                        .collect())
                });

            mock_storage
                .expect_ee_account_state()
                .returning(|block_or_slot| {
                    if let OLBlockOrSlot::Block(block_id) = block_or_slot {
                        if block_id.as_ref()[0] >= 7 {
                            return Ok(None);
                        }
                        let slot = block_id.as_ref()[0] as u64;
                        Ok(Some(make_state_at_block(slot, slot as u8, slot as u8)))
                    } else {
                        Ok(None)
                    }
                });

            let result = find_fork_point(&mock_storage, &mock_client, 5, 10, 3)
                .await
                .unwrap();

            assert!(result.is_some());
            assert_eq!(result.unwrap().ol_slot(), 6);
        }

        #[tokio::test]
        async fn test_propagates_client_error() {
            let mock_storage = MockStorage::new();
            let mut mock_client = MockOlClient::new();

            mock_client
                .expect_block_commitments_in_range()
                .times(1)
                .returning(|_, _| Err(crate::traits::error::OlClientError::network("test error")));

            let result = find_fork_point(&mock_storage, &mock_client, 100, 110, 50).await;

            assert!(result.is_err());
        }

        #[tokio::test]
        async fn test_propagates_storage_error() {
            let mut mock_storage = MockStorage::new();
            let mut mock_client = MockOlClient::new();

            mock_client
                .expect_block_commitments_in_range()
                .returning(|start, end| {
                    Ok((start..=end)
                        .map(|slot| make_block_commitment(slot, slot as u8))
                        .collect())
                });

            mock_storage
                .expect_ee_account_state()
                .times(1)
                .returning(|_| Err(crate::traits::error::StorageError::database("test error")));

            let result = find_fork_point(&mock_storage, &mock_client, 100, 110, 50).await;

            assert!(result.is_err());
        }
    }

    mod rollback_to_fork_point_tests {
        use super::*;

        #[tokio::test]
        async fn test_performs_rollback_and_builds_state() {
            let mut mock_storage = MockStorage::new();

            let fork_state = make_state_at_block(100, 1, 1);
            let ol_status = OlChainStatus {
                latest: make_block_commitment(110, 2),
                confirmed: make_block_commitment(105, 3),
                finalized: make_block_commitment(100, 1),
            };

            mock_storage
                .expect_rollback_ee_account_state()
                .times(1)
                .withf(|slot| *slot == 100)
                .returning(|_| Ok(()));

            mock_storage
                .expect_ee_account_state()
                .times(2)
                .returning(|block_or_slot| match block_or_slot {
                    OLBlockOrSlot::Block(block_id) => {
                        let slot = block_id.as_ref()[0] as u64;
                        Ok(Some(make_state_at_block(slot, slot as u8, slot as u8)))
                    }
                    OLBlockOrSlot::Slot(slot) => {
                        Ok(Some(make_state_at_block(slot, slot as u8, slot as u8)))
                    }
                });

            let mut state = OlTrackerState::new(
                make_state_at_block(110, 2, 2),
                make_state_at_block(105, 3, 3),
                make_state_at_block(100, 1, 1),
            );
            let result =
                rollback_to_fork_point(&mut state, &mock_storage, &fork_state, &ol_status).await;

            assert!(result.is_ok());
            assert_eq!(state.best_ee_state(), fork_state.ee_state());
        }

        #[tokio::test]
        async fn test_propagates_rollback_error() {
            let mut mock_storage = MockStorage::new();

            let fork_state = make_state_at_block(100, 1, 1);
            let ol_status = OlChainStatus {
                latest: make_block_commitment(110, 2),
                confirmed: make_block_commitment(105, 3),
                finalized: make_block_commitment(100, 1),
            };

            mock_storage
                .expect_ee_account_state()
                .times(2)
                .returning(|block_or_slot| match block_or_slot {
                    OLBlockOrSlot::Block(block_id) => {
                        let slot = block_id.as_ref()[0] as u64;
                        Ok(Some(make_state_at_block(slot, slot as u8, slot as u8)))
                    }
                    OLBlockOrSlot::Slot(slot) => {
                        Ok(Some(make_state_at_block(slot, slot as u8, slot as u8)))
                    }
                });

            mock_storage
                .expect_rollback_ee_account_state()
                .times(1)
                .returning(|_| {
                    Err(crate::traits::error::StorageError::database(
                        "rollback failed",
                    ))
                });

            let mut state = OlTrackerState::new(
                make_state_at_block(110, 2, 2),
                make_state_at_block(105, 3, 3),
                make_state_at_block(100, 1, 1),
            );

            let result =
                rollback_to_fork_point(&mut state, &mock_storage, &fork_state, &ol_status).await;

            assert!(result.is_err());
        }
    }

    mod handle_reorg_tests {
        use std::sync::Arc;

        use alloy_primitives::B256;
        use strata_acct_types::AccountId;
        use strata_identifiers::Buf32;
        use tokio::sync::watch;

        use super::*;
        use crate::{config::AlpenEeParams, ol_tracker::ConsensusHeads};

        fn make_test_params(genesis_slot: u64) -> AlpenEeParams {
            AlpenEeParams::new(
                AccountId::new([0; 32]),
                B256::ZERO,
                B256::ZERO,
                genesis_slot,
                Buf32::from([0; 32]).into(),
            )
        }

        fn make_test_ctx(
            storage: MockStorage,
            ol_client: MockOlClient,
            genesis_slot: u64,
            reorg_fetch_size: u64,
        ) -> OlTrackerCtx<MockStorage, MockOlClient> {
            let (ee_state_tx, _) = watch::channel(make_ee_state([0; 32]));
            let (consensus_tx, _) = watch::channel(ConsensusHeads {
                confirmed: [0; 32],
                finalized: [0; 32],
            });

            let params = make_test_params(genesis_slot);

            OlTrackerCtx {
                storage: Arc::new(storage),
                ol_client: Arc::new(ol_client),
                params: Arc::new(params),
                ee_state_tx,
                consensus_tx,
                max_blocks_fetch: 10,
                poll_wait_ms: 100,
                reorg_fetch_size,
            }
        }

        #[tokio::test]
        async fn test_successful_reorg() {
            let mut mock_storage = MockStorage::new();
            let mut mock_client = MockOlClient::new();

            mock_client.expect_chain_status().times(1).returning(|| {
                Ok(OlChainStatus {
                    latest: make_block_commitment(110, 2),
                    confirmed: make_block_commitment(105, 3),
                    finalized: make_block_commitment(100, 1),
                })
            });

            mock_client
                .expect_block_commitments_in_range()
                .returning(|start, end| {
                    Ok((start..=end)
                        .map(|slot| make_block_commitment(slot, slot as u8))
                        .collect())
                });

            mock_storage
                .expect_ee_account_state()
                .returning(|block_or_slot| match block_or_slot {
                    OLBlockOrSlot::Block(block_id) => {
                        let slot = block_id.as_ref()[0] as u64;
                        if slot >= 108 {
                            return Ok(None);
                        }
                        Ok(Some(make_state_at_block(slot, slot as u8, slot as u8)))
                    }
                    OLBlockOrSlot::Slot(slot) => {
                        Ok(Some(make_state_at_block(slot, slot as u8, slot as u8)))
                    }
                });

            mock_storage
                .expect_rollback_ee_account_state()
                .times(1)
                .withf(|slot| *slot == 107)
                .returning(|_| Ok(()));

            let ctx = make_test_ctx(mock_storage, mock_client, 100, 50);
            let mut state = OlTrackerState::new(
                make_state_at_block(110, 2, 2),
                make_state_at_block(105, 3, 3),
                make_state_at_block(100, 1, 1),
            );

            let result = handle_reorg(&mut state, &ctx).await;

            assert!(result.is_ok());
            let mut expected_bytes = [0u8; 32];
            expected_bytes[0] = 107;
            assert_eq!(
                state.best_ee_state().last_exec_blkid(),
                strata_acct_types::Hash::from(expected_bytes)
            );
        }

        #[tokio::test]
        async fn test_fails_when_no_fork_point_found() {
            let mut mock_storage = MockStorage::new();
            let mut mock_client = MockOlClient::new();

            mock_client.expect_chain_status().times(1).returning(|| {
                Ok(OlChainStatus {
                    latest: make_block_commitment(110, 2),
                    confirmed: make_block_commitment(105, 3),
                    finalized: make_block_commitment(100, 1),
                })
            });

            mock_client
                .expect_block_commitments_in_range()
                .returning(|start, end| {
                    Ok((start..=end)
                        .map(|slot| make_block_commitment(slot, slot as u8))
                        .collect())
                });

            mock_storage
                .expect_ee_account_state()
                .returning(|_| Ok(None));

            let ctx = make_test_ctx(mock_storage, mock_client, 100, 50);
            let mut state = OlTrackerState::new(
                make_state_at_block(110, 2, 2),
                make_state_at_block(105, 3, 3),
                make_state_at_block(100, 1, 1),
            );

            let result = handle_reorg(&mut state, &ctx).await;

            assert!(result.is_err());
            let error = result.unwrap_err();
            // Check that it's specifically the NoForkPointFound error
            assert!(matches!(error, OlTrackerError::NoForkPointFound { .. }));
            // Verify the error message contains key info
            assert!(error.to_string().contains("no fork point found"));
            assert!(error.to_string().contains("100")); // genesis slot
        }

        #[tokio::test]
        async fn test_propagates_chain_status_error() {
            let mock_storage = MockStorage::new();
            let mut mock_client = MockOlClient::new();

            mock_client.expect_chain_status().times(1).returning(|| {
                Err(crate::traits::error::OlClientError::network(
                    "network error",
                ))
            });

            let ctx = make_test_ctx(mock_storage, mock_client, 100, 50);
            let mut state = OlTrackerState::new(
                make_state_at_block(110, 2, 2),
                make_state_at_block(105, 3, 3),
                make_state_at_block(100, 1, 1),
            );

            let result = handle_reorg(&mut state, &ctx).await;

            assert!(result.is_err());
        }

        #[tokio::test]
        async fn test_reorg_with_small_fetch_size() {
            let mut mock_storage = MockStorage::new();
            let mut mock_client = MockOlClient::new();

            mock_client.expect_chain_status().times(1).returning(|| {
                Ok(OlChainStatus {
                    latest: make_block_commitment(110, 2),
                    confirmed: make_block_commitment(105, 3),
                    finalized: make_block_commitment(100, 1),
                })
            });

            mock_client
                .expect_block_commitments_in_range()
                .returning(|start, end| {
                    Ok((start..=end)
                        .map(|slot| make_block_commitment(slot, slot as u8))
                        .collect())
                });

            mock_storage
                .expect_ee_account_state()
                .returning(|block_or_slot| match block_or_slot {
                    OLBlockOrSlot::Block(block_id) => {
                        let slot = block_id.as_ref()[0] as u64;
                        if slot >= 104 {
                            return Ok(None);
                        }
                        Ok(Some(make_state_at_block(slot, slot as u8, slot as u8)))
                    }
                    OLBlockOrSlot::Slot(slot) => {
                        Ok(Some(make_state_at_block(slot, slot as u8, slot as u8)))
                    }
                });

            mock_storage
                .expect_rollback_ee_account_state()
                .times(1)
                .withf(|slot| *slot == 103)
                .returning(|_| Ok(()));

            let ctx = make_test_ctx(mock_storage, mock_client, 100, 5);
            let mut state = OlTrackerState::new(
                make_state_at_block(110, 2, 2),
                make_state_at_block(105, 3, 3),
                make_state_at_block(100, 1, 1),
            );

            let result = handle_reorg(&mut state, &ctx).await;

            assert!(result.is_ok());
        }

        #[tokio::test]
        async fn test_state_unchanged_when_build_tracker_state_fails() {
            let mut mock_storage = MockStorage::new();
            let mut mock_client = MockOlClient::new();

            mock_client.expect_chain_status().times(1).returning(|| {
                Ok(OlChainStatus {
                    latest: make_block_commitment(110, 2),
                    confirmed: make_block_commitment(105, 3),
                    finalized: make_block_commitment(100, 1),
                })
            });

            mock_client
                .expect_block_commitments_in_range()
                .returning(|start, end| {
                    Ok((start..=end)
                        .map(|slot| make_block_commitment(slot, slot as u8))
                        .collect())
                });

            // Fork point found at slot 107
            // But build_tracker_state will fail when querying for confirmed/finalized blocks
            mock_storage
                .expect_ee_account_state()
                .times(..)
                .returning(|block_or_slot| match block_or_slot {
                    OLBlockOrSlot::Block(block_id) => {
                        let id_byte = block_id.as_ref()[0];
                        if id_byte >= 108 {
                            return Ok(None);
                        }
                        // Simulate failure when reading finalized block (which has id=1, slot=100)
                        if id_byte == 1 {
                            return Err(crate::traits::error::StorageError::database(
                                "simulated storage read failure for finalized block",
                            ));
                        }
                        let slot = id_byte as u64;
                        Ok(Some(make_state_at_block(slot, id_byte, id_byte)))
                    }
                    OLBlockOrSlot::Slot(slot) => {
                        Ok(Some(make_state_at_block(slot, slot as u8, slot as u8)))
                    }
                });

            // DB rollback should NOT be called because build_tracker_state fails first
            mock_storage.expect_rollback_ee_account_state().times(0);

            let ctx = make_test_ctx(mock_storage, mock_client, 100, 50);
            let initial_state = OlTrackerState::new(
                make_state_at_block(110, 2, 2),
                make_state_at_block(105, 3, 3),
                make_state_at_block(100, 1, 1),
            );
            let mut state = initial_state.clone();

            let result = handle_reorg(&mut state, &ctx).await;

            // Reorg should fail
            assert!(result.is_err());
            assert!(result
                .unwrap_err()
                .to_string()
                .contains("simulated storage read failure"));

            // State should remain unchanged
            assert_eq!(state.best_ol_block().slot(), 110);
            assert_eq!(state.best_ol_block().blkid().as_ref()[0], 2);
        }

        #[tokio::test]
        async fn test_state_unchanged_when_rollback_fails() {
            let mut mock_storage = MockStorage::new();
            let mut mock_client = MockOlClient::new();

            mock_client.expect_chain_status().times(1).returning(|| {
                Ok(OlChainStatus {
                    latest: make_block_commitment(110, 2),
                    confirmed: make_block_commitment(105, 3),
                    finalized: make_block_commitment(100, 1),
                })
            });

            mock_client
                .expect_block_commitments_in_range()
                .returning(|start, end| {
                    Ok((start..=end)
                        .map(|slot| make_block_commitment(slot, slot as u8))
                        .collect())
                });

            // Fork point found at slot 107, and build_tracker_state succeeds
            mock_storage
                .expect_ee_account_state()
                .returning(|block_or_slot| match block_or_slot {
                    OLBlockOrSlot::Block(block_id) => {
                        let slot = block_id.as_ref()[0] as u64;
                        if slot >= 108 {
                            return Ok(None);
                        }
                        Ok(Some(make_state_at_block(slot, slot as u8, slot as u8)))
                    }
                    OLBlockOrSlot::Slot(slot) => {
                        Ok(Some(make_state_at_block(slot, slot as u8, slot as u8)))
                    }
                });

            // DB rollback fails
            mock_storage
                .expect_rollback_ee_account_state()
                .times(1)
                .withf(|slot| *slot == 107)
                .returning(|_| {
                    Err(crate::traits::error::StorageError::database(
                        "simulated rollback failure",
                    ))
                });

            let ctx = make_test_ctx(mock_storage, mock_client, 100, 50);
            let initial_state = OlTrackerState::new(
                make_state_at_block(110, 2, 2),
                make_state_at_block(105, 3, 3),
                make_state_at_block(100, 1, 1),
            );
            let mut state = initial_state.clone();

            let result = handle_reorg(&mut state, &ctx).await;

            // Reorg should fail
            assert!(result.is_err());
            assert!(result
                .unwrap_err()
                .to_string()
                .contains("simulated rollback failure"));

            // State should remain unchanged - this is the critical atomicity guarantee
            assert_eq!(state.best_ol_block().slot(), 110);
            assert_eq!(state.best_ol_block().blkid().as_ref()[0], 2);
        }

        #[tokio::test]
        async fn test_state_not_mutated_when_find_fork_fails() {
            let mut mock_storage = MockStorage::new();
            let mut mock_client = MockOlClient::new();

            mock_client.expect_chain_status().times(1).returning(|| {
                Ok(OlChainStatus {
                    latest: make_block_commitment(110, 2),
                    confirmed: make_block_commitment(105, 3),
                    finalized: make_block_commitment(100, 1),
                })
            });

            // No fork point found - all blocks return None
            mock_client
                .expect_block_commitments_in_range()
                .returning(|start, end| {
                    Ok((start..=end)
                        .map(|slot| make_block_commitment(slot, slot as u8))
                        .collect())
                });

            mock_storage
                .expect_ee_account_state()
                .returning(|_| Ok(None));

            // DB rollback should NOT be called
            mock_storage.expect_rollback_ee_account_state().times(0);

            let ctx = make_test_ctx(mock_storage, mock_client, 100, 50);
            let initial_state = OlTrackerState::new(
                make_state_at_block(110, 2, 2),
                make_state_at_block(105, 3, 3),
                make_state_at_block(100, 1, 1),
            );
            let mut state = initial_state.clone();

            let result = handle_reorg(&mut state, &ctx).await;

            // Reorg should fail
            assert!(result.is_err());

            // State should remain unchanged
            assert_eq!(state.best_ol_block().slot(), 110);
            assert_eq!(state.best_ol_block().blkid().as_ref()[0], 2);
        }
    }
}
