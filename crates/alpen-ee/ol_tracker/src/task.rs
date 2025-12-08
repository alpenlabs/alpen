use std::time::Duration;

use alpen_ee_common::{
    chain_status_checked, EeAccountStateAtEpoch, OLChainStatus, OLClient, Storage,
};
use strata_ee_acct_runtime::apply_update_operation_unconditionally;
use strata_ee_acct_types::EeAccountState;
use strata_identifiers::EpochCommitment;
use strata_snark_acct_types::UpdateInputData;
use tracing::{debug, error, warn};

use crate::{
    ctx::OLTrackerCtx,
    error::{OLTrackerError, Result},
    reorg::handle_reorg,
    state::{build_tracker_state, OLTrackerState},
};

pub(crate) async fn ol_tracker_task<TStorage, TOLClient>(
    mut state: OLTrackerState,
    ctx: OLTrackerCtx<TStorage, TOLClient>,
) where
    TStorage: Storage,
    TOLClient: OLClient,
{
    loop {
        tokio::time::sleep(Duration::from_millis(ctx.poll_wait_ms)).await;

        match track_ol_state(&state, ctx.ol_client.as_ref(), ctx.max_epochs_fetch).await {
            Ok(TrackOLAction::Extend(epoch_operations, chain_status)) => {
                if let Err(error) =
                    handle_extend_ee_state(&epoch_operations, &chain_status, &mut state, &ctx).await
                {
                    handle_tracker_error(error, "extend ee state");
                }
            }
            Ok(TrackOLAction::Reorg) => {
                if let Err(error) = handle_reorg(&mut state, &ctx).await {
                    handle_tracker_error(error, "reorg");
                }
            }
            Ok(TrackOLAction::Noop) => {}
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
fn handle_tracker_error(error: impl Into<OLTrackerError>, context: &str) {
    let error = error.into();

    if error.is_fatal() {
        panic!("{}", error.panic_message());
    } else {
        error!(%error, %context, "recoverable error in ol tracker");
    }
}

#[derive(Debug)]
pub(crate) struct OLEpochOperations {
    pub epoch: EpochCommitment,
    pub operations: Vec<UpdateInputData>,
}

#[derive(Debug)]
pub(crate) enum TrackOLAction {
    /// Extend local view of the OL chain with new epochs.
    /// TODO: stream
    Extend(Vec<OLEpochOperations>, OLChainStatus),
    /// Local tip not present in OL chain, need to resolve local view.
    Reorg,
    /// Local tip is synced with OL chain, nothing to do.
    Noop,
}

pub(crate) async fn track_ol_state(
    state: &OLTrackerState,
    ol_client: &impl OLClient,
    max_epochs_fetch: u32,
) -> Result<TrackOLAction> {
    // can be changed to subscribe to ol changes, with timeout
    let ol_status = chain_status_checked(ol_client).await?;

    let best_ol_confirmed = &ol_status.confirmed;
    let best_ol_epoch = best_ol_confirmed.epoch();
    let best_local_epoch = state.best_ol_epoch().epoch();

    debug!(%best_local_epoch, %best_ol_epoch, "check best ol confirmed epoch");

    if best_ol_epoch < best_local_epoch {
        warn!(
            "local view of chain is ahead of OL, should not typically happen; local: {}; ol: {}",
            best_local_epoch, best_ol_confirmed
        );
        return Ok(TrackOLAction::Noop);
    }

    if best_ol_epoch == best_local_epoch {
        if best_ol_confirmed.last_blkid() != state.best_ol_epoch().last_blkid() {
            warn!(
                epoch = %best_ol_epoch,
                ol = %best_ol_confirmed.last_blkid(),
                local = %state.best_ol_epoch().last_blkid(),
                "detect chain mismatch; trigger reorg"
            );
            return Ok(TrackOLAction::Reorg);
        } else {
            // local view is in sync with OL, nothing to do
            return Ok(TrackOLAction::Noop);
        };
    }

    if best_ol_epoch > best_local_epoch {
        // local chain is behind ol's confirmed view, we can fetch next epochs and extend local
        // view.
        let fetch_epochs_count = best_ol_epoch
            .saturating_sub(best_local_epoch)
            .min(max_epochs_fetch);

        // Fetch epoch summaries for new epochs
        let mut epoch_operations = Vec::new();
        let mut expected_prev = *state.best_ol_epoch();

        for count in 1..=fetch_epochs_count {
            let epoch_num = best_local_epoch + count;
            let epoch_summary = ol_client.epoch_summary(epoch_num).await?;

            // Verify chain continuity
            if epoch_summary.prev_epoch() != &expected_prev {
                if epoch_num == best_local_epoch + 1 {
                    // First new epoch's prev doesn't match our local state.
                    // -> our local view is invalid
                    warn!(
                        epoch = %epoch_num,
                        expected_prev = %expected_prev,
                        actual_prev = %epoch_summary.prev_epoch(),
                        "local chain state invalid; trigger reorg"
                    );
                    return Ok(TrackOLAction::Reorg);
                } else {
                    // Subsequent epoch doesn't chain properly - remote reorg during fetch
                    // Process what we have so far and handle reorg in next cycle
                    debug!(
                        epoch = %epoch_num,
                        expected_prev = %expected_prev,
                        actual_prev = %epoch_summary.prev_epoch(),
                        "chain discontinuity detected; stopping batch fetch"
                    );
                    break;
                }
            }

            epoch_operations.push(OLEpochOperations {
                epoch: *epoch_summary.epoch(),
                operations: epoch_summary.updates().to_vec(),
            });

            // Update expected_prev for next iteration
            expected_prev = *epoch_summary.epoch();
        }

        // maybe stream all missing epochs ?
        return Ok(TrackOLAction::Extend(epoch_operations, ol_status));
    }

    unreachable!("There should not be a valid case that is not covered above")
}

pub(crate) fn apply_epoch_operations(
    state: &mut EeAccountState,
    epoch_operations: &[UpdateInputData],
) -> Result<()> {
    for op in epoch_operations {
        apply_update_operation_unconditionally(state, op)
            .map_err(|e| OLTrackerError::Other(e.to_string()))?;
    }

    Ok(())
}

async fn handle_extend_ee_state<TStorage, TOLClient>(
    epoch_operations: &[OLEpochOperations],
    chain_status: &OLChainStatus,
    state: &mut OLTrackerState,
    ctx: &OLTrackerCtx<TStorage, TOLClient>,
) -> Result<()>
where
    TStorage: Storage,
    TOLClient: OLClient,
{
    for epoch_op in epoch_operations {
        let OLEpochOperations {
            epoch: ol_epoch,
            operations,
        } = epoch_op;

        let mut ee_state = state.best_ee_state().clone();

        // 1. Apply all operations in the epoch to update local ee account state.
        apply_epoch_operations(&mut ee_state, operations).map_err(|error| {
            error!(
                epoch = %ol_epoch.epoch(),
                %error,
                "failed to apply ol epoch operation"
            );
            error
        })?;

        // 2. build next tracker state
        let next_state = build_tracker_state(
            EeAccountStateAtEpoch::new(*ol_epoch, ee_state.clone()),
            chain_status,
            ctx.storage.as_ref(),
        )
        .await?;

        // 3. Atomically persist corresponding ee state for this ol epoch.
        ctx.storage
            .store_ee_account_state(ol_epoch, &ee_state)
            .await
            .map_err(|error| {
                error!(
                    epoch = %ol_epoch.epoch(),
                    %error,
                    "failed to store ee account state"
                );
                error
            })?;

        // 4. update local state
        *state = next_state;

        // 5. notify watchers
        ctx.notify_ol_status_update(state.get_ol_status());
        ctx.notify_consensus_update(state.get_consensus_heads());
    }

    Ok(())
}

// #[cfg(test)]
// mod tests {
//     use alpen_ee_common::{MockOLClient, OLChainStatus};
//     use strata_acct_types::BitcoinAmount;
//     use strata_identifiers::{Buf32, OLBlockCommitment};

//     use super::*;

//     /// Helper to create a block commitment for testing
//     fn make_block_commitment(slot: u64, id: u8) -> OLBlockCommitment {
//         let mut bytes = [0u8; 32];
//         bytes[0] = id;
//         OLBlockCommitment::new(slot, Buf32::new(bytes).into())
//     }

//     /// Helper to create a test tracker state
//     fn make_test_state(slot: u64, id: u8) -> OLTrackerState {
//         let block_state = EeAccountStateAtBlock::new(
//             make_block_commitment(slot, id),
//             EeAccountState::new([0u8; 32], BitcoinAmount::zero(), vec![], vec![]),
//         );
//         OLTrackerState::new(block_state.clone(), block_state.clone(), block_state)
//     }

//     mod apply_block_operations_tests {
//         use super::*;

//         #[test]
//         fn test_apply_empty_operations() {
//             let mut state = EeAccountState::new([0u8; 32], BitcoinAmount::zero(), vec![], vec![]);
//             let operations: Vec<UpdateInputData> = vec![];

//             let result = apply_block_operations(&mut state, &operations);

//             assert!(result.is_ok());
//         }
//     }

//     mod track_ol_state_tests {
//         use super::*;

//         #[tokio::test]
//         async fn test_noop_when_local_ahead() {
//             let state = make_test_state(100, 1);
//             let mut mock_client = MockOLClient::new();

//             mock_client.expect_chain_status().times(1).returning(|| {
//                 Ok(OLChainStatus {
//                     latest: make_block_commitment(50, 1), // OL chain is behind
//                     confirmed: make_block_commitment(50, 1),
//                     finalized: make_block_commitment(50, 1),
//                 })
//             });

//             let result = track_ol_state(&state, &mock_client, 10).await.unwrap();

//             assert!(matches!(result, TrackOLAction::Noop));
//         }

//         #[tokio::test]
//         async fn test_noop_when_synced() {
//             let state = make_test_state(100, 1);
//             let mut mock_client = MockOLClient::new();

//             mock_client.expect_chain_status().times(1).returning(|| {
//                 Ok(OLChainStatus {
//                     latest: make_block_commitment(100, 1), // Same slot and ID
//                     confirmed: make_block_commitment(100, 1),
//                     finalized: make_block_commitment(100, 1),
//                 })
//             });

//             let result = track_ol_state(&state, &mock_client, 10).await.unwrap();

//             assert!(matches!(result, TrackOLAction::Noop));
//         }

//         #[tokio::test]
//         async fn test_reorg_when_same_slot_different_block_id() {
//             let state = make_test_state(100, 1);
//             let mut mock_client = MockOLClient::new();

//             mock_client.expect_chain_status().times(1).returning(|| {
//                 Ok(OLChainStatus {
//                     latest: make_block_commitment(100, 2), // Same slot, different ID
//                     confirmed: make_block_commitment(100, 2),
//                     finalized: make_block_commitment(100, 2),
//                 })
//             });

//             let result = track_ol_state(&state, &mock_client, 10).await.unwrap();

//             assert!(matches!(result, TrackOLAction::Reorg));
//         }

//         #[tokio::test]
//         async fn test_reorg_when_local_block_not_in_chain() {
//             let state = make_test_state(100, 1);
//             let mut mock_client = MockOLClient::new();

//             mock_client.expect_chain_status().times(1).returning(|| {
//                 Ok(OLChainStatus {
//                     latest: make_block_commitment(101, 2),
//                     confirmed: make_block_commitment(101, 2),
//                     finalized: make_block_commitment(101, 2),
//                 })
//             });

//             // Note: block_commitments_in_range_checked calls block_commitments_in_range
//             mock_client
//                 .expect_block_commitments_in_range()
//                 .times(1)
//                 .withf(|start, end| *start == 100 && *end == 101)
//                 .returning(|_, _| {
//                     Ok(vec![
//                         make_block_commitment(100, 99), // Different ID at slot 100
//                         make_block_commitment(101, 2),
//                     ])
//                 });

//             let result = track_ol_state(&state, &mock_client, 10).await.unwrap();

//             assert!(matches!(result, TrackOLAction::Reorg));
//         }

//         #[tokio::test]
//         async fn test_extend_with_one_new_block() {
//             let state = make_test_state(100, 100);
//             let mut mock_client = MockOLClient::new();

//             mock_client.expect_chain_status().times(1).returning(|| {
//                 Ok(OLChainStatus {
//                     latest: make_block_commitment(101, 101),
//                     confirmed: make_block_commitment(101, 101),
//                     finalized: make_block_commitment(101, 101),
//                 })
//             });

//             // Mock for block_commitments_in_range_checked
//             mock_client
//                 .expect_block_commitments_in_range()
//                 .times(1)
//                 .withf(|start, end| *start == 100 && *end == 101)
//                 .returning(|_, _| {
//                     Ok(vec![
//                         make_block_commitment(100, 100), // Local block
//                         make_block_commitment(101, 101), // New block
//                     ])
//                 });

//             // Mock for get_update_operations_for_blocks_checked
//             mock_client
//                 .expect_get_update_operations_for_blocks()
//                 .times(1)
//                 .returning(|blocks| Ok(vec![vec![]; blocks.len()]));

//             let result = track_ol_state(&state, &mock_client, 10).await.unwrap();

//             match result {
//                 TrackOLAction::Extend(ops, _status) => {
//                     assert_eq!(ops.len(), 1);
//                     assert_eq!(ops[0].block.slot(), 101);
//                 }
//                 _ => panic!("Expected Extend action"),
//             }
//         }

//         #[tokio::test]
//         async fn test_extend_with_multiple_blocks() {
//             let state = make_test_state(100, 100);
//             let mut mock_client = MockOLClient::new();

//             mock_client.expect_chain_status().times(1).returning(|| {
//                 Ok(OLChainStatus {
//                     latest: make_block_commitment(103, 103),
//                     confirmed: make_block_commitment(103, 103),
//                     finalized: make_block_commitment(103, 103),
//                 })
//             });

//             // Mock for block_commitments_in_range_checked
//             mock_client
//                 .expect_block_commitments_in_range()
//                 .times(1)
//                 .withf(|start, end| *start == 100 && *end == 103)
//                 .returning(|_, _| {
//                     Ok(vec![
//                         make_block_commitment(100, 100),
//                         make_block_commitment(101, 101),
//                         make_block_commitment(102, 102),
//                         make_block_commitment(103, 103),
//                     ])
//                 });

//             // Mock for get_update_operations_for_blocks_checked
//             mock_client
//                 .expect_get_update_operations_for_blocks()
//                 .times(1)
//                 .returning(|blocks| Ok(vec![vec![]; blocks.len()]));

//             let result = track_ol_state(&state, &mock_client, 10).await.unwrap();

//             match result {
//                 TrackOLAction::Extend(ops, _status) => {
//                     assert_eq!(ops.len(), 3);
//                     assert_eq!(ops[0].block.slot(), 101);
//                     assert_eq!(ops[1].block.slot(), 102);
//                     assert_eq!(ops[2].block.slot(), 103);
//                 }
//                 _ => panic!("Expected Extend action"),
//             }
//         }

//         #[tokio::test]
//         async fn test_extend_respects_max_blocks_fetch() {
//             let state = make_test_state(100, 100);
//             let mut mock_client = MockOLClient::new();

//             mock_client.expect_chain_status().times(1).returning(|| {
//                 Ok(OLChainStatus {
//                     latest: make_block_commitment(150, 150), // 50 blocks behind
//                     confirmed: make_block_commitment(150, 150),
//                     finalized: make_block_commitment(150, 150),
//                 })
//             });

//             // Mock for block_commitments_in_range_checked - should cap at 5 blocks
//             mock_client
//                 .expect_block_commitments_in_range()
//                 .times(1)
//                 .withf(|start, end| *start == 100 && *end == 105) // Should cap at 5
//                 .returning(|start, end| {
//                     Ok((start..=end)
//                         .map(|slot| make_block_commitment(slot, slot as u8))
//                         .collect())
//                 });

//             // Mock for get_update_operations_for_blocks_checked
//             mock_client
//                 .expect_get_update_operations_for_blocks()
//                 .times(1)
//                 .returning(|blocks| Ok(vec![vec![]; blocks.len()]));

//             let result = track_ol_state(&state, &mock_client, 5).await.unwrap();

//             match result {
//                 TrackOLAction::Extend(ops, _status) => {
//                     assert_eq!(ops.len(), 5);
//                     assert_eq!(ops[0].block.slot(), 101);
//                     assert_eq!(ops[4].block.slot(), 105);
//                 }
//                 _ => panic!("Expected Extend action"),
//             }
//         }
//     }
// }
