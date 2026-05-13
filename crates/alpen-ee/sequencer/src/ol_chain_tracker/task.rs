use std::sync::Arc;

use alpen_ee_common::{
    get_inbox_messages_checked, ExecBlockStorage, OLBlockData, OLFinalizedStatus, SequencerOLClient,
};
use eyre::eyre;
use strata_identifiers::OLBlockCommitment;
use tokio::{
    select,
    sync::{mpsc, oneshot, watch},
};
use tracing::{error, warn};

use super::state::OLChainTrackerState;
use crate::ol_chain_tracker::state::InboxMessages;

pub(crate) enum OLChainTrackerQuery {
    GetFinalizedBlock(oneshot::Sender<OLBlockCommitment>),
    GetInboxMessages {
        from_slot: u64,
        to_slot: u64,
        response_tx: oneshot::Sender<eyre::Result<InboxMessages>>,
    },
}

pub(crate) async fn ol_chain_tracker_task<
    TClient: SequencerOLClient,
    TStorage: ExecBlockStorage,
>(
    mut chainstatus_rx: watch::Receiver<OLFinalizedStatus>,
    mut query_rx: mpsc::Receiver<OLChainTrackerQuery>,
    mut state: OLChainTrackerState,
    client: Arc<TClient>,
    storage: Arc<TStorage>,
) {
    loop {
        select! {
            chainstatus_changed = chainstatus_rx.changed() => {
                if chainstatus_changed.is_err() {
                    warn!("channel is closed; shutting down");
                    break;
                }
                // we only track inbox messages from finalized blocks to include in block assembly.
                let ol_finalized_status = *chainstatus_rx.borrow_and_update();
                handle_chain_update(ol_finalized_status, &mut state, client.as_ref(), storage.as_ref()).await;
            }
            maybe_query = query_rx.recv() => {
                let Some(query) = maybe_query else {
                    warn!("channel is closed; shutting down");
                    break;
                };
                handle_chain_query(&state, query);
            }
        }
    }
}

fn handle_chain_query(state: &OLChainTrackerState, query: OLChainTrackerQuery) {
    match query {
        OLChainTrackerQuery::GetFinalizedBlock(tx) => {
            let _ = tx.send(state.best_block());
        }
        OLChainTrackerQuery::GetInboxMessages {
            from_slot,
            to_slot,
            response_tx,
        } => {
            let _ = response_tx.send(state.get_inbox_messages(from_slot, to_slot));
        }
    }
}

async fn handle_chain_update(
    ol_finalized_status: OLFinalizedStatus,
    state: &mut OLChainTrackerState,
    client: &impl SequencerOLClient,
    storage: &impl ExecBlockStorage,
) {
    // compare latest finalized block with local chain segment using db. get extend, revert info
    match track_ol_state(state, ol_finalized_status, client).await {
        Ok(TrackAction::Extend(ol_blocks)) => {
            // update tracker state with new blocks
            for OLBlockData {
                commitment,
                inbox_messages,
                next_inbox_msg_idx,
            } in ol_blocks
            {
                if let Err(err) = state.append_block(commitment, inbox_messages, next_inbox_msg_idx)
                {
                    // As blocks are expected to be in order, if one block cannot be appended,
                    // the remaining blocks will also fail.
                    // So skip rest of the updates and retry in in next cycle.
                    error!(
                        %commitment,
                        ?err,
                        "failed to append block to ol chain tracker; skipping update"
                    );
                    return;
                }
            }

            // check if state c
            if let Err(err) = handle_state_pruning(state, ol_finalized_status, storage).await {
                error!(?err, "failed to prune state");
            }
        }
        Ok(TrackAction::Reorg(_next)) => {
            // kill task and trigger app shutdown through TaskManager.
            panic!("Deep reorg detected. Manual resolution required.")
        }
        Err(err) => {
            error!(?err, "failed to track ol state");
            // retry next cycle
            // TODO: unrecoverable error
        }
    };
}

#[derive(Debug)]
enum TrackAction {
    Extend(Vec<OLBlockData>),
    Reorg(OLBlockCommitment),
}

async fn track_ol_state(
    state: &OLChainTrackerState,
    ol_finalized_status: OLFinalizedStatus,
    ol_client: &impl SequencerOLClient,
) -> eyre::Result<TrackAction> {
    let best_ol_block = state.best_block();
    // We only care about finalized ol blocks to use as inputs to block assembly.
    let remote_finalized_ol_block = ol_finalized_status.ol_block;

    if remote_finalized_ol_block == best_ol_block {
        // nothing to do
        return Ok(TrackAction::Extend(vec![]));
    }
    if remote_finalized_ol_block.slot() <= best_ol_block.slot() {
        warn!(
            local = ?best_ol_block,
            remote = ?remote_finalized_ol_block,
            "local finalized OL block ahead of OL"
        );

        return Ok(TrackAction::Reorg(remote_finalized_ol_block));
    }
    if remote_finalized_ol_block.slot() > best_ol_block.slot() {
        let blocks = get_inbox_messages_checked(
            ol_client,
            best_ol_block.slot(),
            remote_finalized_ol_block.slot(),
        )
        .await?;

        let (block_at_finalized_height, blocks) = {
            let mut iter = blocks.into_iter();
            let first = iter.next().expect("checked");

            (first, iter)
        };

        if block_at_finalized_height.commitment != best_ol_block {
            // The block we know to be finalized locally is not present in the OL chain.
            // OL chain has seen a deep reorg.
            // Avoid corrupting local data and exit to await manual resolution.

            warn!(
                local = ?best_ol_block,
                remote = ?block_at_finalized_height.commitment,
                "local finalized OL block not present in OL"
            );

            return Ok(TrackAction::Reorg(block_at_finalized_height.commitment));
        }

        return Ok(TrackAction::Extend(blocks.collect()));
    }

    unreachable!("all valid cases should have been handled above");
}

async fn handle_state_pruning(
    state: &mut OLChainTrackerState,
    finalized_status: OLFinalizedStatus,
    storage: &impl ExecBlockStorage,
) -> eyre::Result<()> {
    let finalized_ee_block = finalized_status.last_ee_block;
    // find last ol block whose data was included in this ee block
    let exec_package = storage
        .get_exec_block(finalized_ee_block)
        .await?
        .ok_or(eyre!(
            "finalized exec block not found: {finalized_ee_block:?}"
        ))?;

    let included_ol_block = exec_package.ol_block();
    state.prune_blocks(*included_ol_block)
}

#[cfg(test)]
mod tests {
    use alpen_ee_common::{MockExecBlockStorage, MockSequencerOLClient, OLClientError};

    use super::*;
    use crate::ol_chain_tracker::test_utils::{
        create_block_data_chain, create_mock_exec_record, create_ol_block_chain, make_block,
        make_block_data, make_block_with_id,
    };

    mod track_ol_state_tests {
        use super::*;

        fn make_finalized_status(ol_block: OLBlockCommitment) -> OLFinalizedStatus {
            OLFinalizedStatus {
                ol_block,
                last_ee_block: Default::default(),
            }
        }

        #[tokio::test]
        async fn returns_empty_extend_when_synced() {
            // Scenario: Local and remote are at the same finalized block
            //
            // Local:  [...] -> [slot=10] (finalized)
            // Remote: [...] -> [slot=10] (finalized)
            //
            // Expected: Extend(vec![]) - nothing to do

            let block = make_block(10);
            let state = OLChainTrackerState::new_empty(block, 0);
            let ol_status = make_finalized_status(block);

            let mock_client = MockSequencerOLClient::new();

            let result = track_ol_state(&state, ol_status, &mock_client)
                .await
                .unwrap();

            match result {
                TrackAction::Extend(blocks) => assert!(blocks.is_empty()),
                TrackAction::Reorg(_) => panic!("expected Extend, got Reorg"),
            }
        }

        #[tokio::test]
        async fn returns_reorg_when_local_ahead_by_slot() {
            // Scenario: Local finalized slot is ahead of remote finalized slot
            //
            // Local:  [...] -> [slot=15] (finalized)
            // Remote: [...] -> [slot=10] (finalized)
            //
            // Expected: Reorg - local is ahead, indicates deep reorg on OL

            let local_block = make_block(15);
            let remote_block = make_block(10);

            let state = OLChainTrackerState::new_empty(local_block, 0);
            let ol_status = make_finalized_status(remote_block);

            let mock_client = MockSequencerOLClient::new();

            let result = track_ol_state(&state, ol_status, &mock_client)
                .await
                .unwrap();

            match result {
                TrackAction::Reorg(block) => assert_eq!(block, remote_block),
                TrackAction::Extend(_) => panic!("expected Reorg, got Extend"),
            }
        }

        #[tokio::test]
        async fn returns_reorg_when_same_slot_different_id() {
            // Scenario: Same slot but different block ID
            //
            // Local:  [...] -> [slot=10, id=0xAA] (finalized)
            // Remote: [...] -> [slot=10, id=0xBB] (finalized)
            //
            // Expected: Reorg - blocks diverged at the same height

            let local_block = make_block_with_id(10, 0xAA);
            let remote_block = make_block_with_id(10, 0xBB);

            let state = OLChainTrackerState::new_empty(local_block, 0);
            let ol_status = make_finalized_status(remote_block);

            let mock_client = MockSequencerOLClient::new();

            let result = track_ol_state(&state, ol_status, &mock_client)
                .await
                .unwrap();

            // Since remote.slot() == local.slot() but blocks differ,
            // the equality check fails and we hit the <= branch -> Reorg
            match result {
                TrackAction::Reorg(block) => assert_eq!(block, remote_block),
                TrackAction::Extend(_) => panic!("expected Reorg, got Extend"),
            }
        }

        #[tokio::test]
        async fn returns_extend_with_new_blocks() {
            // Scenario: Remote is ahead by 3 blocks, chain is consistent
            //
            // Local:  [...] -> [slot=10] (finalized)
            // Remote: [...] -> [slot=10] -> [slot=11] -> [slot=12] -> [slot=13] (finalized)
            //
            // Expected: Extend with blocks 11, 12, 13

            let local_block = make_block(10);
            let remote_block = make_block(13);

            let state = OLChainTrackerState::new_empty(local_block, 0);
            let ol_status = make_finalized_status(remote_block);

            // Create block chain from slot 10 to 13
            let ol_blocks = create_ol_block_chain(10, 4); // slots 10, 11, 12, 13
            let block_data = create_block_data_chain(&ol_blocks, 0);

            let mut mock_client = MockSequencerOLClient::new();
            mock_client
                .expect_get_inbox_messages()
                .withf(|min, max| *min == 10 && *max == 13)
                .times(1)
                .returning(move |_, _| Ok(block_data.clone()));

            let result = track_ol_state(&state, ol_status, &mock_client)
                .await
                .unwrap();

            match result {
                TrackAction::Extend(blocks) => {
                    // Should return blocks 11, 12, 13 (excluding the first one which is local)
                    assert_eq!(blocks.len(), 3);
                    assert_eq!(blocks[0].commitment.slot(), 11);
                    assert_eq!(blocks[1].commitment.slot(), 12);
                    assert_eq!(blocks[2].commitment.slot(), 13);
                }
                TrackAction::Reorg(_) => panic!("expected Extend, got Reorg"),
            }
        }

        #[tokio::test]
        async fn returns_reorg_on_deep_reorg_block_mismatch() {
            // Scenario: Remote is ahead but has different block at local's finalized slot
            //
            // Local:  [...] -> [slot=10, id=0xAA] (finalized)
            // Remote: [...] -> [slot=10, id=0xBB] -> [slot=11] -> ... -> [slot=13] (finalized)
            //                  ^ different block!
            //
            // Expected: Reorg - deep reorg detected

            let local_block = make_block_with_id(10, 0xAA);
            let remote_block = make_block(13);

            let state = OLChainTrackerState::new_empty(local_block, 0);
            let ol_status = make_finalized_status(remote_block);

            // Remote has a different block at slot 10
            let remote_block_at_10 = make_block_with_id(10, 0xBB);
            let block_data = vec![
                make_block_data(remote_block_at_10, vec![], 0),
                make_block_data(make_block(11), vec![], 0),
                make_block_data(make_block(12), vec![], 0),
                make_block_data(make_block(13), vec![], 0),
            ];

            let mut mock_client = MockSequencerOLClient::new();
            mock_client
                .expect_get_inbox_messages()
                .times(1)
                .returning(move |_, _| Ok(block_data.clone()));

            let result = track_ol_state(&state, ol_status, &mock_client)
                .await
                .unwrap();

            match result {
                TrackAction::Reorg(block) => {
                    // Should return the mismatched block from remote
                    assert_eq!(block, remote_block_at_10);
                }
                TrackAction::Extend(_) => panic!("expected Reorg, got Extend"),
            }
        }

        #[tokio::test]
        async fn propagates_client_error() {
            // Scenario: OL client fails to fetch inbox messages
            //
            // Local:  [...] -> [slot=10] (finalized)
            // Remote: [...] -> [slot=15] (finalized)
            //
            // Expected: Error propagated from client

            let local_block = make_block(10);
            let remote_block = make_block(15);

            let state = OLChainTrackerState::new_empty(local_block, 0);
            let ol_status = make_finalized_status(remote_block);

            let mut mock_client = MockSequencerOLClient::new();
            mock_client
                .expect_get_inbox_messages()
                .times(1)
                .returning(|_, _| Err(OLClientError::network("connection refused")));

            let result = track_ol_state(&state, ol_status, &mock_client).await;

            assert!(result.is_err());
            assert!(result
                .unwrap_err()
                .to_string()
                .contains("connection refused"));
        }
    }

    mod handle_state_pruning_tests {
        use strata_acct_types::Hash;
        use strata_identifiers::Buf32;

        use super::*;

        fn make_finalized_status(
            ol_block: OLBlockCommitment,
            ee_block_hash: Hash,
        ) -> OLFinalizedStatus {
            OLFinalizedStatus {
                ol_block,
                last_ee_block: ee_block_hash,
            }
        }

        #[tokio::test]
        async fn prunes_to_included_ol_block() {
            // Scenario: State has blocks, finalized EE block references an earlier OL block
            //
            // OL State: base=[slot=10] -> [slot=11] -> [slot=12] -> [slot=13] -> [slot=14] ->
            // [slot=15] Finalized EE block references OL block at slot=13
            //
            // Expected: Prune up to slot=13, leaving slots 14, 15 in state

            let base = make_block(10);
            let mut state = OLChainTrackerState::new_empty(base, 0);

            let blocks = create_ol_block_chain(11, 5); // slots 11, 12, 13, 14, 15
            for block in &blocks {
                state.append_block(*block, vec![], 0).unwrap();
            }

            // OL block at slot 13 was included in the finalized exec block
            let included_ol_block = blocks[2]; // slot 13
            let ee_block_hash = Hash::from(Buf32::new([0x42; 32]));

            let exec_record = create_mock_exec_record(included_ol_block);

            let mut mock_storage = MockExecBlockStorage::new();
            mock_storage
                .expect_get_exec_block()
                .withf(move |h| *h == ee_block_hash)
                .times(1)
                .returning(move |_| Ok(Some(exec_record.clone())));

            let ol_status = make_finalized_status(make_block(15), ee_block_hash);

            handle_state_pruning(&mut state, ol_status, &mock_storage)
                .await
                .unwrap();

            // After pruning to slot 13, only slots 14 and 15 should remain
            assert_eq!(state.best_block().slot(), 15);
            // The base should now be slot 13
            let messages = state.get_inbox_messages(11, 13);
            assert!(messages.is_ok());
            assert!(messages.unwrap().messages().is_empty()); // These were pruned or clamped
        }

        #[tokio::test]
        async fn errors_when_exec_block_not_found() {
            // Scenario: Storage doesn't have the finalized exec block
            //
            // OL State: base=[slot=10] -> [slot=11]
            // Finalized EE block hash points to missing block
            //
            // Expected: Error "finalized exec block not found"

            let base = make_block(10);
            let mut state = OLChainTrackerState::new_empty(base, 0);

            state.append_block(make_block(11), vec![], 0).unwrap();

            let ee_block_hash = Hash::from(Buf32::new([0x99; 32]));

            let mut mock_storage = MockExecBlockStorage::new();
            mock_storage
                .expect_get_exec_block()
                .times(1)
                .returning(|_| Ok(None));

            let ol_status = make_finalized_status(make_block(15), ee_block_hash);

            let result = handle_state_pruning(&mut state, ol_status, &mock_storage).await;

            assert!(result.is_err());
            assert!(result
                .unwrap_err()
                .to_string()
                .contains("finalized exec block not found"));
        }

        #[tokio::test]
        async fn errors_when_storage_fails() {
            // Scenario: Storage returns an error when fetching exec block
            //
            // OL State: base=[slot=10] -> [slot=11]
            // Storage fails with DB error
            //
            // Expected: Error propagated from storage

            let base = make_block(10);
            let mut state = OLChainTrackerState::new_empty(base, 0);

            state.append_block(make_block(11), vec![], 0).unwrap();

            let ee_block_hash = Hash::from(Buf32::new([0x99; 32]));

            let mut mock_storage = MockExecBlockStorage::new();
            mock_storage
                .expect_get_exec_block()
                .times(1)
                .returning(|_| {
                    Err(alpen_ee_common::StorageError::Other(eyre::eyre!(
                        "db connection failed"
                    )))
                });

            let ol_status = make_finalized_status(make_block(15), ee_block_hash);

            let result = handle_state_pruning(&mut state, ol_status, &mock_storage).await;

            assert!(result.is_err());
        }
    }
}
