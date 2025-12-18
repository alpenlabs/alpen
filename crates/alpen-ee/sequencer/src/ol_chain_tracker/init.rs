use alpen_ee_common::{get_inbox_messages_checked, ExecBlockStorage, SequencerOLClient};
use eyre::eyre;
use tracing::error;

use crate::OLChainTrackerState;

/// Initializes tracker state by syncing from local storage and the OL client.
pub async fn init_ol_chain_tracker_state<TStorage: ExecBlockStorage, TClient: SequencerOLClient>(
    storage: &TStorage,
    ol_client: &TClient,
) -> eyre::Result<OLChainTrackerState> {
    // last finalized block known to EE sequencer locally
    let finalized_exec_block = storage
        .best_finalized_block()
        .await?
        .ok_or(eyre!("finalized block missing"))?;
    let local_finalized_ol_block = *finalized_exec_block.ol_block();

    // chain status according to OL
    // TODO: retry
    let ol_chain_status = ol_client.chain_status().await?;
    let remote_finalized_ol_block = ol_chain_status.finalized().to_block_commitment();

    if remote_finalized_ol_block.slot() < local_finalized_ol_block.slot() {
        // Block height that is considered finalized locally is not considered finalized on OL.
        //
        // Either a deep reorg has occurred on OL,
        // or a significant mismatch between OL and EE.
        // In either case, exit to avoid corrupting local data and await manual resolution.
        error!(
            local = ?local_finalized_ol_block,
            remote = ?remote_finalized_ol_block,
            "local finalized OL block ahead of OL"
        );
        return Err(eyre!(
            "local finalized state is ahead of connected OL's finalized slot"
        ));
    }

    // TODO: retry
    // TODO: chunk calls by slot range
    let blocks = get_inbox_messages_checked(
        ol_client,
        local_finalized_ol_block.slot(),
        remote_finalized_ol_block.slot(),
    )
    .await?;

    let (block_at_finalized_height, blocks) = {
        let mut iter = blocks.into_iter();
        // Safe: get_inbox_messages_checked guarantees (max_slot - min_slot + 1) >= 1 blocks.
        let first = iter.next().expect("at least one block guaranteed");

        (first, iter)
    };

    if block_at_finalized_height.commitment != local_finalized_ol_block {
        // The block we know to be finalized locally is not present in the OL chain.
        // OL chain has seen a deep reorg.
        // Avoid corrupting local data and exit to await manual resolution.
        error!(
            local = ?local_finalized_ol_block,
            remote = ?block_at_finalized_height.commitment,
            "local finalized OL block not present in OL"
        );

        return Err(eyre!(
            "local finalized state not present in OL chain. Deep reorg detected."
        ));
    }

    let mut state = OLChainTrackerState::new_empty(
        local_finalized_ol_block,
        block_at_finalized_height.next_inbox_msg_idx,
    );

    // Everything looks ok now. Build local state.
    for block in blocks {
        state.append_block(
            block.commitment,
            block.inbox_messages,
            block.next_inbox_msg_idx,
        )?;
    }

    Ok(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    mod init_ol_chain_tracker_state_tests {
        use alpen_ee_common::{
            MockExecBlockStorage, MockSequencerOLClient, OLChainStatus, OLClientError,
        };

        use super::*;
        use crate::ol_chain_tracker::test_utils::{
            create_block_data_chain, create_mock_exec_record, make_block_data, make_block_with_id,
            make_chain_status,
        };

        // =========================================================================
        // Test Helpers
        // =========================================================================

        /// Sets up mock storage to return the given exec record as best finalized block.
        fn setup_mock_storage_finalized(
            mock_storage: &mut MockExecBlockStorage,
            exec_record: alpen_ee_common::ExecBlockRecord,
        ) {
            mock_storage
                .expect_best_finalized_block()
                .times(1)
                .returning(move || Ok(Some(exec_record.clone())));
        }

        /// Sets up mock OL client to return the given chain status.
        fn setup_mock_client_chain_status(
            mock_client: &mut MockSequencerOLClient,
            status: OLChainStatus,
        ) {
            mock_client
                .expect_chain_status()
                .times(1)
                .returning(move || Ok(status));
        }

        /// Sets up mock OL client to return inbox messages for the given block data.
        fn setup_mock_client_inbox_messages(
            mock_client: &mut MockSequencerOLClient,
            block_data: Vec<alpen_ee_common::OLBlockData>,
        ) {
            mock_client
                .expect_get_inbox_messages()
                .times(1)
                .returning(move |_, _| Ok(block_data.clone()));
        }

        // =========================================================================
        // Tests
        // =========================================================================

        #[tokio::test]
        async fn returns_empty_state_when_synced() {
            // Scenario: Local and remote are at the same finalized block
            //
            // Local chain:   [...] -> [slot=10, id=10] (finalized)
            // Remote chain:  [...] -> [slot=10, id=10] (finalized)
            //
            // Expected: Empty state with base at slot 10

            let finalized_block = make_block_with_id(10, 10);
            let exec_record = create_mock_exec_record(finalized_block);
            let chain_status = make_chain_status(finalized_block);
            // When local == remote, get_inbox_messages_checked(10, 10) is called
            // which returns a single block (the finalized block itself)
            let block_data = vec![make_block_data(finalized_block, vec![], 0)];

            let mut mock_storage = MockExecBlockStorage::new();
            let mut mock_client = MockSequencerOLClient::new();

            setup_mock_storage_finalized(&mut mock_storage, exec_record);
            setup_mock_client_chain_status(&mut mock_client, chain_status);
            setup_mock_client_inbox_messages(&mut mock_client, block_data);

            let state = init_ol_chain_tracker_state(&mock_storage, &mock_client)
                .await
                .unwrap();

            assert_eq!(state.best_block(), finalized_block);
            assert!(state.blocks().is_empty());
        }

        #[tokio::test]
        async fn builds_state_from_new_blocks() {
            // Scenario: Remote is ahead of local by 3 blocks
            //
            // Local chain:   [...] -> [slot=10, id=10] (finalized)
            // Remote chain:  [...] -> [slot=10, id=10] -> [slot=11] -> [slot=12] -> [slot=13]
            // (finalized)
            //
            // Expected: State with base at slot 10, blocks 11-13 tracked

            let local_finalized = make_block_with_id(10, 10);
            let remote_finalized = make_block_with_id(13, 13);

            // Create block chain from slot 10 to 13
            // Use make_block_with_id to ensure block at slot 10 matches local_finalized
            let ol_blocks: Vec<_> = (10..=13)
                .map(|slot| make_block_with_id(slot, slot as u8))
                .collect();
            let block_data = create_block_data_chain(&ol_blocks, 0);

            let exec_record = create_mock_exec_record(local_finalized);
            let chain_status = make_chain_status(remote_finalized);

            let mut mock_storage = MockExecBlockStorage::new();
            let mut mock_client = MockSequencerOLClient::new();

            setup_mock_storage_finalized(&mut mock_storage, exec_record);
            setup_mock_client_chain_status(&mut mock_client, chain_status);
            setup_mock_client_inbox_messages(&mut mock_client, block_data);

            let state = init_ol_chain_tracker_state(&mock_storage, &mock_client)
                .await
                .unwrap();

            // Base should be local finalized (slot 10)
            assert_eq!(state.base().slot(), 10);
            // Should have 3 new blocks tracked (11, 12, 13)
            assert_eq!(state.blocks().len(), 3);
            assert_eq!(state.best_block().slot(), 13);

            // Verify messages were stored
            let messages = state.get_inbox_messages(11, 13).unwrap();
            assert_eq!(messages.messages().len(), 3);
        }

        #[tokio::test]
        async fn errors_when_finalized_block_missing() {
            // Scenario: Storage has no finalized block
            //
            // Local chain:   (empty)
            // Remote chain:  [...] -> [slot=10] (finalized)
            //
            // Expected: Error "finalized block missing"

            let mut mock_storage = MockExecBlockStorage::new();
            let mock_client = MockSequencerOLClient::new();

            mock_storage
                .expect_best_finalized_block()
                .returning(|| Ok(None));

            let result = init_ol_chain_tracker_state(&mock_storage, &mock_client).await;

            assert!(result.is_err());
            assert!(result
                .unwrap_err()
                .to_string()
                .contains("finalized block missing"));
        }

        #[tokio::test]
        async fn errors_when_local_ahead_of_remote() {
            // Scenario: Local finalized slot is ahead of remote finalized slot
            //
            // Local chain:   [...] -> [slot=15, id=15] (finalized)
            // Remote chain:  [...] -> [slot=10, id=10] (finalized)
            //
            // Expected: Error about local being ahead

            let local_finalized = make_block_with_id(15, 15);
            let remote_finalized = make_block_with_id(10, 10);

            let exec_record = create_mock_exec_record(local_finalized);
            let chain_status = make_chain_status(remote_finalized);

            let mut mock_storage = MockExecBlockStorage::new();
            let mut mock_client = MockSequencerOLClient::new();

            setup_mock_storage_finalized(&mut mock_storage, exec_record);
            setup_mock_client_chain_status(&mut mock_client, chain_status);

            let result = init_ol_chain_tracker_state(&mock_storage, &mock_client).await;

            assert!(result.is_err());
            assert!(result
                .unwrap_err()
                .to_string()
                .contains("local finalized state is ahead"));
        }

        #[tokio::test]
        async fn errors_on_deep_reorg() {
            // Scenario: Same slot but different block ID (deep reorg)
            //
            // Local chain:   [...] -> [slot=10, id=0xAA] (finalized)
            // Remote chain:  [...] -> [slot=10, id=0xBB] -> [slot=11] (finalized)
            //                         ^ different block at same slot!
            //
            // Expected: Error "Deep reorg detected"

            let local_finalized = make_block_with_id(10, 0xAA);
            let remote_finalized = make_block_with_id(11, 11);

            // Remote returns different block at slot 10
            let remote_block_at_10 = make_block_with_id(10, 0xBB);
            let remote_block_at_11 = make_block_with_id(11, 11);
            let block_data = vec![
                make_block_data(remote_block_at_10, vec![], 0),
                make_block_data(remote_block_at_11, vec![], 0),
            ];

            let exec_record = create_mock_exec_record(local_finalized);
            let chain_status = make_chain_status(remote_finalized);

            let mut mock_storage = MockExecBlockStorage::new();
            let mut mock_client = MockSequencerOLClient::new();

            setup_mock_storage_finalized(&mut mock_storage, exec_record);
            setup_mock_client_chain_status(&mut mock_client, chain_status);
            setup_mock_client_inbox_messages(&mut mock_client, block_data);

            let result = init_ol_chain_tracker_state(&mock_storage, &mock_client).await;

            assert!(result.is_err());
            assert!(result
                .unwrap_err()
                .to_string()
                .contains("Deep reorg detected"));
        }

        #[tokio::test]
        async fn errors_when_chain_status_fails() {
            // Scenario: OL client fails to return chain status
            //
            // Expected: Error propagated from client

            let local_finalized = make_block_with_id(10, 10);
            let exec_record = create_mock_exec_record(local_finalized);

            let mut mock_storage = MockExecBlockStorage::new();
            let mut mock_client = MockSequencerOLClient::new();

            setup_mock_storage_finalized(&mut mock_storage, exec_record);
            mock_client
                .expect_chain_status()
                .returning(|| Err(OLClientError::network("connection refused")));

            let result = init_ol_chain_tracker_state(&mock_storage, &mock_client).await;

            assert!(result.is_err());
            assert!(result
                .unwrap_err()
                .to_string()
                .contains("connection refused"));
        }

        #[tokio::test]
        async fn errors_when_get_inbox_messages_fails() {
            // Scenario: OL client fails to return inbox messages
            //
            // Local chain:   [...] -> [slot=10, id=10] (finalized)
            // Remote chain:  [...] -> [slot=10] -> [slot=11] (finalized)
            //
            // Expected: Error propagated from client

            let local_finalized = make_block_with_id(10, 10);
            let remote_finalized = make_block_with_id(11, 11);

            let exec_record = create_mock_exec_record(local_finalized);
            let chain_status = make_chain_status(remote_finalized);

            let mut mock_storage = MockExecBlockStorage::new();
            let mut mock_client = MockSequencerOLClient::new();

            setup_mock_storage_finalized(&mut mock_storage, exec_record);
            setup_mock_client_chain_status(&mut mock_client, chain_status);
            mock_client
                .expect_get_inbox_messages()
                .returning(|_, _| Err(OLClientError::network("timeout fetching messages")));

            let result = init_ol_chain_tracker_state(&mock_storage, &mock_client).await;

            assert!(result.is_err());
            assert!(result
                .unwrap_err()
                .to_string()
                .contains("timeout fetching messages"));
        }
    }
}
