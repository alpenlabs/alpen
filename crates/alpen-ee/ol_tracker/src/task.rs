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

#[cfg(test)]
mod tests {
    use alpen_ee_common::{MockOLClient, OLChainStatus, OLEpochSummary};
    use strata_acct_types::BitcoinAmount;

    use super::*;
    use crate::test_utils::*;

    mod apply_epoch_operations_tests {
        use strata_acct_types::Hash;

        use super::*;

        #[test]
        fn test_apply_empty_operations() {
            // Scenario: Apply empty operations list
            // Expected: State unchanged, returns Ok
            let mut state =
                EeAccountState::new(Hash::new([0u8; 32]), BitcoinAmount::zero(), vec![], vec![]);
            let operations: Vec<UpdateInputData> = vec![];

            let result = apply_epoch_operations(&mut state, &operations);

            assert!(result.is_ok());
        }
    }

    mod track_ol_state_tests {
        use super::*;

        #[tokio::test]
        async fn test_noop_when_local_ahead() {
            // Scenario: Local chain is ahead of OL confirmed chain
            // Local state:    epoch 5 with terminal block ID 105
            // Remote chain:   confirmed epoch 3 (behind local)
            // Expected:       Noop (unusual state, but handled gracefully)

            let chain = create_epochs(&[100, 101, 102, 103, 104, 105]);
            let state = OLTrackerState::new(chain[5].clone(), chain[0].clone());

            let mut mock_client = MockOLClient::new();

            mock_client.expect_chain_status().times(1).returning(|| {
                Ok(OLChainStatus {
                    latest: make_block_commitment(30, 103),
                    confirmed: make_epoch_commitment(3, 30, 103),
                    finalized: make_epoch_commitment(0, 0, 100),
                })
            });

            let result = track_ol_state(&state, &mock_client, 10).await.unwrap();

            assert!(matches!(result, TrackOLAction::Noop));
        }

        #[tokio::test]
        async fn test_noop_when_synced() {
            // Scenario: Local chain is in sync with OL confirmed chain
            // Local state:    epoch 3 with terminal block ID 103
            // Remote chain:   confirmed epoch 3 with same terminal block ID 103
            // Expected:       Noop (already synced)

            let chain = create_epochs(&[100, 101, 102, 103]);
            let state = OLTrackerState::new(chain[3].clone(), chain[0].clone());

            let mut mock_client = MockOLClient::new();

            mock_client.expect_chain_status().times(1).returning(|| {
                Ok(OLChainStatus {
                    latest: make_block_commitment(30, 103),
                    confirmed: make_epoch_commitment(3, 30, 103),
                    finalized: make_epoch_commitment(0, 0, 100),
                })
            });

            let result = track_ol_state(&state, &mock_client, 10).await.unwrap();

            assert!(matches!(result, TrackOLAction::Noop));
        }

        #[tokio::test]
        async fn test_reorg_when_same_epoch_different_terminal_block() {
            // Scenario: Same epoch but different terminal block ID (chain mismatch)
            // Local state:    epoch 3 with terminal block ID 103
            // Remote chain:   confirmed epoch 3 with terminal block ID 199 (different!)
            // Expected:       Reorg triggered

            let chain = create_epochs(&[100, 101, 102, 103]);
            let state = OLTrackerState::new(chain[3].clone(), chain[0].clone());

            let mut mock_client = MockOLClient::new();

            mock_client.expect_chain_status().times(1).returning(|| {
                Ok(OLChainStatus {
                    latest: make_block_commitment(30, 199),
                    confirmed: make_epoch_commitment(3, 30, 199), // Same epoch, different block ID
                    finalized: make_epoch_commitment(0, 0, 100),
                })
            });

            let result = track_ol_state(&state, &mock_client, 10).await.unwrap();

            assert!(matches!(result, TrackOLAction::Reorg));
        }

        #[tokio::test]
        async fn test_reorg_when_first_new_epoch_prev_mismatch() {
            // Scenario: First new epoch's prev doesn't match local state (reorg detected)
            // Local chain:    [100, 101, 102, 103] (epochs 0-3)
            // Remote chain:   [100, 101, 102, 199, 104] (epochs 0-4, diverged at epoch 3)
            // Local state:    epoch 3 with terminal block ID 103
            // Remote epoch 4's prev has block ID 199 (not 103)
            // Expected:       Reorg triggered

            let local_chain = create_epochs(&[100, 101, 102, 103]);
            let remote_chain = create_epochs(&[100, 101, 102, 199, 104]);

            let state = OLTrackerState::new(local_chain[3].clone(), local_chain[0].clone());

            let mut mock_client = MockOLClient::new();

            mock_client.expect_chain_status().times(1).returning(|| {
                Ok(OLChainStatus {
                    latest: make_block_commitment(40, 104),
                    confirmed: make_epoch_commitment(4, 40, 104),
                    finalized: make_epoch_commitment(0, 0, 100),
                })
            });

            setup_mock_client_with_chain(&mut mock_client, remote_chain);

            let result = track_ol_state(&state, &mock_client, 10).await.unwrap();

            assert!(matches!(result, TrackOLAction::Reorg));
        }

        #[tokio::test]
        async fn test_extend_with_new_epochs() {
            // Scenario: Multiple new epochs to sync
            // Local chain:    [100, 101, 102] (epochs 0-2)
            // Remote chain:   [100, 101, 102, 103, 104, 105] (epochs 0-5)
            // Local state:    epoch 2 with terminal block ID 102
            // Expected:       Extend with epochs 3, 4, 5

            let local_chain = create_epochs(&[100, 101, 102]);
            let remote_chain = create_epochs(&[100, 101, 102, 103, 104, 105]);

            let state = OLTrackerState::new(local_chain[2].clone(), local_chain[0].clone());

            let mut mock_client = MockOLClient::new();

            mock_client.expect_chain_status().times(1).returning(|| {
                Ok(OLChainStatus {
                    latest: make_block_commitment(50, 105),
                    confirmed: make_epoch_commitment(5, 50, 105),
                    finalized: make_epoch_commitment(0, 0, 100),
                })
            });

            setup_mock_client_with_chain(&mut mock_client, remote_chain);

            let result = track_ol_state(&state, &mock_client, 10).await.unwrap();

            match result {
                TrackOLAction::Extend(ops, _status) => {
                    assert_eq!(ops.len(), 3);
                    assert_eq!(ops[0].epoch.epoch(), 3);
                    assert_eq!(ops[1].epoch.epoch(), 4);
                    assert_eq!(ops[2].epoch.epoch(), 5);
                }
                _ => panic!("Expected Extend action"),
            }
        }

        #[tokio::test]
        async fn test_extend_respects_max_epochs_fetch() {
            // Scenario: Many epochs behind but capped by max_epochs_fetch
            // Local chain:    [100] (epoch 0)
            // Remote chain:   [100, 101, 102, ..., 110] (epochs 0-10)
            // Local state:    epoch 0 with terminal block ID 100
            // max_epochs_fetch: 3
            // Expected:       Extend with only epochs 1, 2, 3

            let local_chain = create_epochs(&[100]);
            let remote_chain =
                create_epochs(&[100, 101, 102, 103, 104, 105, 106, 107, 108, 109, 110]);

            let state = OLTrackerState::new(local_chain[0].clone(), local_chain[0].clone());

            let mut mock_client = MockOLClient::new();

            mock_client.expect_chain_status().times(1).returning(|| {
                Ok(OLChainStatus {
                    latest: make_block_commitment(100, 110),
                    confirmed: make_epoch_commitment(10, 100, 110),
                    finalized: make_epoch_commitment(0, 0, 100),
                })
            });

            setup_mock_client_with_chain(&mut mock_client, remote_chain);

            let result = track_ol_state(&state, &mock_client, 3).await.unwrap();

            match result {
                TrackOLAction::Extend(ops, _status) => {
                    assert_eq!(ops.len(), 3);
                    assert_eq!(ops[0].epoch.epoch(), 1);
                    assert_eq!(ops[1].epoch.epoch(), 2);
                    assert_eq!(ops[2].epoch.epoch(), 3);
                }
                _ => panic!("Expected Extend action"),
            }
        }

        #[tokio::test]
        async fn test_extend_stops_on_chain_discontinuity() {
            // Scenario: Chain discontinuity detected during batch fetch (remote reorg mid-fetch)
            // Local chain:    [100, 101, 102] (epochs 0-2)
            // Local state:    epoch 2 with terminal block ID 102
            // Remote returns: epoch 3 (prev=102), epoch 4 (prev=103), epoch 5 (prev=199!)
            // Epoch 5's prev (199) doesn't match the epoch 4 (104) we just fetched
            // Expected:       Extend with epochs 3, 4 only (stops at discontinuity)
            //
            // Note: This simulates a remote reorg happening mid-fetch, requiring manual mock
            // setup since the helper only produces internally consistent chains.

            let local_chain = create_epochs(&[100, 101, 102]);
            let state = OLTrackerState::new(local_chain[2].clone(), local_chain[0].clone());

            let mut mock_client = MockOLClient::new();

            mock_client.expect_chain_status().times(1).returning(|| {
                Ok(OLChainStatus {
                    latest: make_block_commitment(50, 105),
                    confirmed: make_epoch_commitment(5, 50, 105),
                    finalized: make_epoch_commitment(0, 0, 100),
                })
            });

            mock_client
                .expect_epoch_summary()
                .withf(|epoch| *epoch == 3)
                .returning(|_| {
                    Ok(OLEpochSummary::new(
                        make_epoch_commitment(3, 30, 103),
                        make_epoch_commitment(2, 20, 102),
                        vec![],
                    ))
                });

            mock_client
                .expect_epoch_summary()
                .withf(|epoch| *epoch == 4)
                .returning(|_| {
                    Ok(OLEpochSummary::new(
                        make_epoch_commitment(4, 40, 104),
                        make_epoch_commitment(3, 30, 103),
                        vec![],
                    ))
                });

            mock_client
                .expect_epoch_summary()
                .withf(|epoch| *epoch == 5)
                .returning(|_| {
                    Ok(OLEpochSummary::new(
                        make_epoch_commitment(5, 50, 105),
                        make_epoch_commitment(4, 40, 199), /* Discontinuity: prev doesn't match
                                                            * epoch 4 */
                        vec![],
                    ))
                });

            let result = track_ol_state(&state, &mock_client, 10).await.unwrap();

            match result {
                TrackOLAction::Extend(ops, _status) => {
                    assert_eq!(ops.len(), 2);
                    assert_eq!(ops[0].epoch.epoch(), 3);
                    assert_eq!(ops[1].epoch.epoch(), 4);
                }
                _ => panic!("Expected Extend action"),
            }
        }

        #[tokio::test]
        async fn test_propagates_client_error() {
            // Scenario: OL client returns error when fetching chain status
            // Expected:       Error propagated

            let chain = create_epochs(&[100, 101, 102]);
            let state = OLTrackerState::new(chain[2].clone(), chain[0].clone());

            let mut mock_client = MockOLClient::new();

            mock_client
                .expect_chain_status()
                .times(1)
                .returning(|| Err(alpen_ee_common::OLClientError::network("network error")));

            let result = track_ol_state(&state, &mock_client, 10).await;

            assert!(result.is_err());
        }
    }
}
