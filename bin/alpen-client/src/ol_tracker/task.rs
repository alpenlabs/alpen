use std::{sync::Arc, time::Duration};

use strata_ee_acct_runtime::apply_update_operation_unconditionally;
use strata_ee_acct_types::EeAccountState;
use strata_identifiers::OLBlockCommitment;
use strata_snark_acct_types::UpdateOperationUnconditionalData;
use tokio::sync::watch;
use tracing::{debug, error, warn};

use crate::{
    ol_tracker::OlTrackerState,
    traits::{
        ol_client::{
            block_commitments_in_range_checked, get_update_operations_for_blocks_checked, OlClient,
        },
        storage::Storage,
    },
};

/// Default number of Ol blocks to process in one cycle
pub(crate) const DEFAULT_MAX_BLOCKS_FETCH: u64 = 10;
/// Default ms to wait between ol polls
pub(crate) const DEFAULT_POLL_WAIT_MS: u64 = 100;

pub(crate) struct OlTrackerCtx<TStorage, TOlClient> {
    pub(crate) storage: Arc<TStorage>,
    pub(crate) ol_client: Arc<TOlClient>,
    pub(crate) ee_state_tx: watch::Sender<EeAccountState>,
    pub(crate) max_blocks_fetch: u64,
    pub(crate) poll_wait_ms: u64,
}

pub(crate) async fn ol_tracker_task<TStorage, TOlClient>(
    mut state: OlTrackerState,
    ctx: OlTrackerCtx<TStorage, TOlClient>,
) where
    TStorage: Storage,
    TOlClient: OlClient,
{
    loop {
        tokio::time::sleep(Duration::from_millis(ctx.poll_wait_ms)).await;

        match track_ol_state(&state, ctx.ol_client.as_ref(), ctx.max_blocks_fetch).await {
            Ok(TrackOlAction::Extend(block_operations)) => {
                if let Err(error) =
                    handle_extend_ee_state(&block_operations, &mut state, &ctx).await
                {
                    error!(%error, "failed to extend ee state");
                }
            }
            Ok(TrackOlAction::Reorg) => {
                handle_reorg(&mut state, &ctx).await;
            }
            Ok(TrackOlAction::Noop) => {}
            Err(error) => {
                error!(%error, "failed to track ol state");
            }
        }
    }
}

#[derive(Debug)]
pub(crate) struct OlBlockOperations {
    pub(crate) block: OLBlockCommitment,
    pub(crate) operations: Vec<UpdateOperationUnconditionalData>,
}

#[derive(Debug)]
pub(crate) enum TrackOlAction {
    /// Extend local view of the OL chain with new blocks.
    /// TODO: stream
    Extend(Vec<OlBlockOperations>),
    /// Local tip not present in OL chain, need to resolve local view.
    Reorg,
    /// Local tip is synced with OL chain, nothing to do.
    Noop,
}

pub(crate) async fn track_ol_state(
    state: &OlTrackerState,
    ol_client: &impl OlClient,
    max_blocks_fetch: u64,
) -> eyre::Result<TrackOlAction> {
    let ol_status = ol_client.chain_status().await.map_err(|e| eyre::eyre!(e))?;

    let best_ol_block = &ol_status.latest;

    debug!(
        local_slot = state.ol_block.slot(),
        ol_slot = best_ol_block.slot(),
        "check best ol block"
    );

    if best_ol_block.slot() < state.ol_block.slot() {
        // local view of chain is ahead of Ol, should not typically happen
        return Ok(TrackOlAction::Noop);
    }
    if best_ol_block.slot() == state.ol_block.slot() {
        return if best_ol_block.blkid() != state.ol_block.blkid() {
            warn!(slot = best_ol_block.slot(), ol = %best_ol_block.blkid(), local = %state.ol_block.blkid(), "detect chain mismatch; trigger reorg");
            Ok(TrackOlAction::Reorg)
        } else {
            Ok(TrackOlAction::Noop)
        };
    }

    // local chain is behind ol's view, we can fetch next blocks and extend local view.
    let fetch_blocks_count = best_ol_block
        .slot()
        .saturating_sub(state.ol_block.slot())
        .min(max_blocks_fetch);

    // Fetch block commitments in from current local slot.
    // Also fetch height of last known local block to check for reorg.
    let blocks = block_commitments_in_range_checked(
        ol_client,
        state.ol_block.slot(),
        state.ol_block.slot() + fetch_blocks_count,
    )
    .await
    .map_err(|e| eyre::eyre!(e))?;

    let (expected_local_block, new_blocks) = blocks
        .split_first()
        .ok_or_else(|| eyre::eyre!("empty block commitments returned from ol_client"))?;

    // If last block isnt as expected, trigger reorg
    if expected_local_block != &state.ol_block {
        return Ok(TrackOlAction::Reorg);
    }

    let block_ids = new_blocks
        .iter()
        .map(|commitment| commitment.blkid())
        .cloned()
        .collect();

    let operations = get_update_operations_for_blocks_checked(ol_client, block_ids)
        .await
        .map_err(|e| eyre::eyre!(e))?;

    let res = new_blocks
        .iter()
        .cloned()
        .zip(operations)
        .map(|(block, operations)| OlBlockOperations { block, operations })
        .collect();

    Ok(TrackOlAction::Extend(res))
}

pub(crate) fn apply_block_operations(
    state: &mut EeAccountState,
    block_operations: &[UpdateOperationUnconditionalData],
) -> eyre::Result<()> {
    for op in block_operations {
        apply_update_operation_unconditionally(state, op)?;
    }

    Ok(())
}

/// Pure function to update tracker state with new block and ee state.
pub(crate) fn update_tracker_state(
    state: &mut OlTrackerState,
    ol_block: OLBlockCommitment,
    ee_state: EeAccountState,
) {
    state.ol_block = ol_block;
    state.ee_state = ee_state;
}

/// Notify watchers of state update.
pub(crate) fn notify_state_update(sender: &watch::Sender<EeAccountState>, state: &EeAccountState) {
    let _ = sender.send(state.clone());
}

async fn handle_extend_ee_state<TStorage, TOlClient>(
    block_operations: &[OlBlockOperations],
    state: &mut OlTrackerState,
    ctx: &OlTrackerCtx<TStorage, TOlClient>,
) -> eyre::Result<()>
where
    TStorage: Storage,
    TOlClient: OlClient,
{
    for block_op in block_operations {
        let OlBlockOperations {
            block: ol_block,
            operations,
        } = block_op;

        let mut ee_state = state.ee_state.clone();

        // 1. Apply all operations in the block to update local ee account state.
        apply_block_operations(&mut ee_state, &operations).map_err(|error| {
            error!(
                slot = ol_block.slot(),
                %error,
                "failed to apply ol block operation"
            );
            error
        })?;

        // 2. Persist corresponding ee state for every ol block
        ctx.storage
            .store_ee_account_state(ol_block, &ee_state)
            .await
            .map_err(|error| {
                error!(
                    slot = ol_block.slot(),
                    %error,
                    "failed to store ee account state"
                );
                eyre::eyre!(error)
            })?;

        // 3. update local state
        update_tracker_state(state, *ol_block, ee_state.clone());

        // 4. notify watchers
        notify_state_update(&ctx.ee_state_tx, &ee_state);
    }

    Ok(())
}

async fn handle_reorg<TStorage, TOlClient>(
    _state: &mut OlTrackerState,
    _ctx: &OlTrackerCtx<TStorage, TOlClient>,
) where
    TStorage: Storage,
    TOlClient: OlClient,
{
    warn!("handle reorg");
    todo!()
}

#[cfg(test)]
mod tests {
    use strata_acct_types::BitcoinAmount;
    use strata_identifiers::{Buf32, OLBlockCommitment};
    use tokio::sync::watch;

    use super::*;
    use crate::traits::ol_client::{MockOlClient, OlChainStatus};

    /// Helper to create a block commitment for testing
    fn make_block_commitment(slot: u64, id: u8) -> OLBlockCommitment {
        let mut bytes = [0u8; 32];
        bytes[0] = id;
        OLBlockCommitment::new(slot, Buf32::new(bytes).into())
    }

    /// Helper to create a test tracker state
    fn make_test_state(slot: u64, id: u8) -> OlTrackerState {
        OlTrackerState {
            ee_state: EeAccountState::new([0u8; 32], BitcoinAmount::zero(), vec![], vec![]),
            ol_block: make_block_commitment(slot, id),
        }
    }

    mod update_tracker_state_tests {
        use super::*;

        #[test]
        fn test_updates_both_fields() {
            let mut state = make_test_state(10, 1);
            let new_block = make_block_commitment(11, 2);
            let new_ee_state =
                EeAccountState::new([1u8; 32], BitcoinAmount::zero(), vec![], vec![]);

            update_tracker_state(&mut state, new_block, new_ee_state.clone());

            assert_eq!(state.ol_block, new_block);
            assert_eq!(state.ee_state, new_ee_state);
        }
    }

    mod notify_state_update_tests {
        use super::*;

        #[test]
        fn test_notification_sent() {
            let initial_state =
                EeAccountState::new([0u8; 32], BitcoinAmount::zero(), vec![], vec![]);
            let (tx, mut rx) = watch::channel(initial_state.clone());

            let new_state = EeAccountState::new([1u8; 32], BitcoinAmount::zero(), vec![], vec![]);

            notify_state_update(&tx, &new_state);

            // Verify notification was received
            assert_eq!(*rx.borrow_and_update(), new_state);
        }

        #[test]
        fn test_notification_with_no_receivers() {
            let initial_state =
                EeAccountState::new([0u8; 32], BitcoinAmount::zero(), vec![], vec![]);
            let (tx, rx) = watch::channel(initial_state);

            // Drop the receiver
            drop(rx);

            let new_state = EeAccountState::new([1u8; 32], BitcoinAmount::zero(), vec![], vec![]);

            // Should not panic even with no receivers
            notify_state_update(&tx, &new_state);
        }
    }

    mod apply_block_operations_tests {
        use super::*;

        #[test]
        fn test_apply_empty_operations() {
            let mut state = EeAccountState::new([0u8; 32], BitcoinAmount::zero(), vec![], vec![]);
            let operations: Vec<UpdateOperationUnconditionalData> = vec![];

            let result = apply_block_operations(&mut state, &operations);

            assert!(result.is_ok());
        }
    }

    mod track_ol_state_tests {
        use super::*;

        #[tokio::test]
        async fn test_noop_when_local_ahead() {
            let state = make_test_state(100, 1);
            let mut mock_client = MockOlClient::new();

            mock_client.expect_chain_status().times(1).returning(|| {
                Ok(OlChainStatus {
                    latest: make_block_commitment(50, 1), // OL chain is behind
                    confirmed: make_block_commitment(50, 1),
                    finalized: make_block_commitment(50, 1),
                })
            });

            let result = track_ol_state(&state, &mock_client, 10).await.unwrap();

            assert!(matches!(result, TrackOlAction::Noop));
        }

        #[tokio::test]
        async fn test_noop_when_synced() {
            let state = make_test_state(100, 1);
            let mut mock_client = MockOlClient::new();

            mock_client.expect_chain_status().times(1).returning(|| {
                Ok(OlChainStatus {
                    latest: make_block_commitment(100, 1), // Same slot and ID
                    confirmed: make_block_commitment(100, 1),
                    finalized: make_block_commitment(100, 1),
                })
            });

            let result = track_ol_state(&state, &mock_client, 10).await.unwrap();

            assert!(matches!(result, TrackOlAction::Noop));
        }

        #[tokio::test]
        async fn test_reorg_when_same_slot_different_block_id() {
            let state = make_test_state(100, 1);
            let mut mock_client = MockOlClient::new();

            mock_client.expect_chain_status().times(1).returning(|| {
                Ok(OlChainStatus {
                    latest: make_block_commitment(100, 2), // Same slot, different ID
                    confirmed: make_block_commitment(100, 2),
                    finalized: make_block_commitment(100, 2),
                })
            });

            let result = track_ol_state(&state, &mock_client, 10).await.unwrap();

            assert!(matches!(result, TrackOlAction::Reorg));
        }

        #[tokio::test]
        async fn test_reorg_when_local_block_not_in_chain() {
            let state = make_test_state(100, 1);
            let mut mock_client = MockOlClient::new();

            mock_client.expect_chain_status().times(1).returning(|| {
                Ok(OlChainStatus {
                    latest: make_block_commitment(101, 2),
                    confirmed: make_block_commitment(101, 2),
                    finalized: make_block_commitment(101, 2),
                })
            });

            // Note: block_commitments_in_range_checked calls block_commitments_in_range
            mock_client
                .expect_block_commitments_in_range()
                .times(1)
                .withf(|start, end| *start == 100 && *end == 101)
                .returning(|_, _| {
                    Ok(vec![
                        make_block_commitment(100, 99), // Different ID at slot 100
                        make_block_commitment(101, 2),
                    ])
                });

            let result = track_ol_state(&state, &mock_client, 10).await.unwrap();

            assert!(matches!(result, TrackOlAction::Reorg));
        }

        #[tokio::test]
        async fn test_extend_with_one_new_block() {
            let state = make_test_state(100, 100);
            let mut mock_client = MockOlClient::new();

            mock_client.expect_chain_status().times(1).returning(|| {
                Ok(OlChainStatus {
                    latest: make_block_commitment(101, 101),
                    confirmed: make_block_commitment(101, 101),
                    finalized: make_block_commitment(101, 101),
                })
            });

            // Mock for block_commitments_in_range_checked
            mock_client
                .expect_block_commitments_in_range()
                .times(1)
                .withf(|start, end| *start == 100 && *end == 101)
                .returning(|_, _| {
                    Ok(vec![
                        make_block_commitment(100, 100), // Local block
                        make_block_commitment(101, 101), // New block
                    ])
                });

            // Mock for get_update_operations_for_blocks_checked
            mock_client
                .expect_get_update_operations_for_blocks()
                .times(1)
                .returning(|blocks| Ok(vec![vec![]; blocks.len()]));

            let result = track_ol_state(&state, &mock_client, 10).await.unwrap();

            match result {
                TrackOlAction::Extend(ops) => {
                    assert_eq!(ops.len(), 1);
                    assert_eq!(ops[0].block.slot(), 101);
                }
                _ => panic!("Expected Extend action"),
            }
        }

        #[tokio::test]
        async fn test_extend_with_multiple_blocks() {
            let state = make_test_state(100, 100);
            let mut mock_client = MockOlClient::new();

            mock_client.expect_chain_status().times(1).returning(|| {
                Ok(OlChainStatus {
                    latest: make_block_commitment(103, 103),
                    confirmed: make_block_commitment(103, 103),
                    finalized: make_block_commitment(103, 103),
                })
            });

            // Mock for block_commitments_in_range_checked
            mock_client
                .expect_block_commitments_in_range()
                .times(1)
                .withf(|start, end| *start == 100 && *end == 103)
                .returning(|_, _| {
                    Ok(vec![
                        make_block_commitment(100, 100),
                        make_block_commitment(101, 101),
                        make_block_commitment(102, 102),
                        make_block_commitment(103, 103),
                    ])
                });

            // Mock for get_update_operations_for_blocks_checked
            mock_client
                .expect_get_update_operations_for_blocks()
                .times(1)
                .returning(|blocks| Ok(vec![vec![]; blocks.len()]));

            let result = track_ol_state(&state, &mock_client, 10).await.unwrap();

            match result {
                TrackOlAction::Extend(ops) => {
                    assert_eq!(ops.len(), 3);
                    assert_eq!(ops[0].block.slot(), 101);
                    assert_eq!(ops[1].block.slot(), 102);
                    assert_eq!(ops[2].block.slot(), 103);
                }
                _ => panic!("Expected Extend action"),
            }
        }

        #[tokio::test]
        async fn test_extend_respects_max_blocks_fetch() {
            let state = make_test_state(100, 100);
            let mut mock_client = MockOlClient::new();

            mock_client.expect_chain_status().times(1).returning(|| {
                Ok(OlChainStatus {
                    latest: make_block_commitment(150, 150), // 50 blocks behind
                    confirmed: make_block_commitment(150, 150),
                    finalized: make_block_commitment(150, 150),
                })
            });

            // Mock for block_commitments_in_range_checked - should cap at 5 blocks
            mock_client
                .expect_block_commitments_in_range()
                .times(1)
                .withf(|start, end| *start == 100 && *end == 105) // Should cap at 5
                .returning(|start, end| {
                    Ok((start..=end)
                        .map(|slot| make_block_commitment(slot, slot as u8))
                        .collect())
                });

            // Mock for get_update_operations_for_blocks_checked
            mock_client
                .expect_get_update_operations_for_blocks()
                .times(1)
                .returning(|blocks| Ok(vec![vec![]; blocks.len()]));

            let result = track_ol_state(&state, &mock_client, 5).await.unwrap();

            match result {
                TrackOlAction::Extend(ops) => {
                    assert_eq!(ops.len(), 5);
                    assert_eq!(ops[0].block.slot(), 101);
                    assert_eq!(ops[4].block.slot(), 105);
                }
                _ => panic!("Expected Extend action"),
            }
        }
    }
}
