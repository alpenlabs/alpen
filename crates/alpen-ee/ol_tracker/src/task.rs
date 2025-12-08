use std::time::Duration;

use alpen_ee_common::{
    block_commitments_in_range_checked, chain_status_checked,
    get_update_operations_for_blocks_checked, EeAccountStateAtBlock, OlChainStatus, OlClient,
    Storage,
};
use strata_ee_acct_runtime::apply_update_operation_unconditionally;
use strata_ee_acct_types::EeAccountState;
use strata_identifiers::OLBlockCommitment;
use strata_snark_acct_types::UpdateInputData;
use tracing::{debug, error, warn};

use crate::{
    ctx::OlTrackerCtx,
    error::{OlTrackerError, Result},
    reorg::handle_reorg,
    state::{build_tracker_state, OlTrackerState},
};

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
            Ok(TrackOlAction::Extend(block_operations, chain_status)) => {
                if let Err(error) =
                    handle_extend_ee_state(&block_operations, &chain_status, &mut state, &ctx).await
                {
                    handle_tracker_error(error, "extend ee state");
                }
            }
            Ok(TrackOlAction::Reorg) => {
                if let Err(error) = handle_reorg(&mut state, &ctx).await {
                    handle_tracker_error(error, "reorg");
                }
            }
            Ok(TrackOlAction::Noop) => {}
            Err(error) => {
                handle_tracker_error(error, "track ol state");
            }
        }
    }
}

/// Handles OL tracker errors, panicking on non-recoverable errors.
/// Note: reth task manager expects critical tasks to panic, not return an Err.
/// Critical task panics will trigger app shutdown.
///
/// Recoverable errors (network issues, transient DB failures) are logged and allow retry.
/// Non-recoverable errors (no fork point found) cause immediate panic with detailed message.
fn handle_tracker_error(error: impl Into<OlTrackerError>, context: &str) {
    let error = error.into();

    if error.is_fatal() {
        panic!("{}", error.panic_message());
    } else {
        error!(%error, %context, "recoverable error in ol tracker");
    }
}

#[derive(Debug)]
pub(crate) struct OlBlockOperations {
    pub block: OLBlockCommitment,
    pub operations: Vec<UpdateInputData>,
}

#[derive(Debug)]
pub(crate) enum TrackOlAction {
    /// Extend local view of the OL chain with new blocks.
    /// TODO: stream
    Extend(Vec<OlBlockOperations>, OlChainStatus),
    /// Local tip not present in OL chain, need to resolve local view.
    Reorg,
    /// Local tip is synced with OL chain, nothing to do.
    Noop,
}

pub(crate) async fn track_ol_state(
    state: &OlTrackerState,
    ol_client: &impl OlClient,
    max_blocks_fetch: u64,
) -> Result<TrackOlAction> {
    // can be changed to subscribe to ol changes, with timeout
    let ol_status = chain_status_checked(ol_client).await?;

    let best_ol_block = &ol_status.latest;
    let best_ol_slot = best_ol_block.slot();
    let best_local_slot = state.best_ol_block().slot();

    debug!(%best_local_slot, %best_ol_slot, "check best ol block");

    if best_ol_slot < best_local_slot {
        warn!(
            "local view of chain is ahead of Ol, should not typically happen; local: {}; ol: {}",
            best_local_slot, best_ol_block
        );
        return Ok(TrackOlAction::Noop);
    }

    if best_ol_slot == best_local_slot {
        return if best_ol_block.blkid() != state.best_ol_block().blkid() {
            warn!(slot = %best_ol_slot, ol = %best_ol_block.blkid(), local = %state.best_ol_block().blkid(), "detect chain mismatch; trigger reorg");
            Ok(TrackOlAction::Reorg)
        } else {
            // local view is in sync with OL, nothing to do
            Ok(TrackOlAction::Noop)
        };
    }

    if best_ol_slot > best_local_slot {
        // local chain is behind ol's view, we can fetch next blocks and extend local view.
        let fetch_blocks_count = best_ol_block
            .slot()
            .saturating_sub(best_local_slot)
            .min(max_blocks_fetch);

        // Fetch block commitments in ol from current local slot.
        // Also fetch at height of last known local block to check for reorg.
        let blocks = block_commitments_in_range_checked(
            ol_client,
            best_local_slot,
            best_local_slot + fetch_blocks_count,
        )
        .await?;

        let (expected_local_block, new_blocks) = blocks.split_first().ok_or_else(|| {
            OlTrackerError::Other("empty block commitments returned from ol_client".to_string())
        })?;

        // If last block isn't as expected, trigger reorg
        if expected_local_block != state.best_ol_block() {
            return Ok(TrackOlAction::Reorg);
        }

        let block_ids = new_blocks
            .iter()
            .map(|commitment| commitment.blkid())
            .cloned()
            .collect();

        let operations = get_update_operations_for_blocks_checked(ol_client, block_ids).await?;

        let block_operations = new_blocks
            .iter()
            .cloned()
            .zip(operations)
            .map(|(block, operations)| OlBlockOperations { block, operations })
            .collect();

        // maybe stream all missing blocks ?
        return Ok(TrackOlAction::Extend(block_operations, ol_status));
    }

    unreachable!("There should not be a valid case that is not covered above")
}

pub(crate) fn apply_block_operations(
    state: &mut EeAccountState,
    block_operations: &[UpdateInputData],
) -> Result<()> {
    for op in block_operations {
        apply_update_operation_unconditionally(state, op)
            .map_err(|e| OlTrackerError::Other(e.to_string()))?;
    }

    Ok(())
}

async fn handle_extend_ee_state<TStorage, TOlClient>(
    block_operations: &[OlBlockOperations],
    chain_status: &OlChainStatus,
    state: &mut OlTrackerState,
    ctx: &OlTrackerCtx<TStorage, TOlClient>,
) -> Result<()>
where
    TStorage: Storage,
    TOlClient: OlClient,
{
    for block_op in block_operations {
        let OlBlockOperations {
            block: ol_block,
            operations,
        } = block_op;

        let mut ee_state = state.best_ee_state().clone();

        // 1. Apply all operations in the block to update local ee account state.
        apply_block_operations(&mut ee_state, operations).map_err(|error| {
            error!(
                slot = %ol_block.slot(),
                %error,
                "failed to apply ol block operation"
            );
            error
        })?;

        // 2. build next tracker state
        let next_state = build_tracker_state(
            EeAccountStateAtBlock::new(*ol_block, ee_state.clone()),
            chain_status,
            ctx.storage.as_ref(),
        )
        .await?;

        // 3. Atomically persist corresponding ee state for this ol block.
        ctx.storage
            .store_ee_account_state(ol_block, &ee_state)
            .await
            .map_err(|error| {
                error!(
                    slot = %ol_block.slot(),
                    %error,
                    "failed to store ee account state"
                );
                error
            })?;

        // 4. update local state
        *state = next_state;

        // 5. notify watchers
        ctx.notify_state_update(state.best_ee_state());
        ctx.notify_consensus_update(state.get_consensus_heads());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use alpen_ee_common::{MockOlClient, OlChainStatus};
    use strata_acct_types::BitcoinAmount;
    use strata_identifiers::{Buf32, OLBlockCommitment};

    use super::*;

    /// Helper to create a block commitment for testing
    fn make_block_commitment(slot: u64, id: u8) -> OLBlockCommitment {
        let mut bytes = [0u8; 32];
        bytes[0] = id;
        OLBlockCommitment::new(slot, Buf32::new(bytes).into())
    }

    /// Helper to create a test tracker state
    fn make_test_state(slot: u64, id: u8) -> OlTrackerState {
        let block_state = EeAccountStateAtBlock::new(
            make_block_commitment(slot, id),
            EeAccountState::new([0u8; 32], BitcoinAmount::zero(), vec![], vec![]),
        );
        OlTrackerState::new(block_state.clone(), block_state.clone(), block_state)
    }

    mod apply_block_operations_tests {
        use super::*;

        #[test]
        fn test_apply_empty_operations() {
            let mut state = EeAccountState::new([0u8; 32], BitcoinAmount::zero(), vec![], vec![]);
            let operations: Vec<UpdateInputData> = vec![];

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
                TrackOlAction::Extend(ops, _status) => {
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
                TrackOlAction::Extend(ops, _status) => {
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
                TrackOlAction::Extend(ops, _status) => {
                    assert_eq!(ops.len(), 5);
                    assert_eq!(ops[0].block.slot(), 101);
                    assert_eq!(ops[4].block.slot(), 105);
                }
                _ => panic!("Expected Extend action"),
            }
        }
    }
}
