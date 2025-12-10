use std::collections::{HashMap, VecDeque};

use alpen_ee_common::{get_inbox_messages_checked, ExecBlockStorage, SequencerOLClient};
use eyre::eyre;
use strata_identifiers::{OLBlockCommitment, OLBlockId};
use strata_snark_acct_types::MessageEntry;
use tracing::{error, warn};

#[derive(Debug)]
pub struct OLChainTrackerState {
    /// Lowest block being tracked.
    /// The messages upto this block have already been processed.
    base: OLBlockCommitment,
    /// blocks whose messages have not been processed.
    blocks: VecDeque<OLBlockCommitment>,
    /// messages in the blocks.
    data: HashMap<OLBlockId, Vec<MessageEntry>>,
}

impl OLChainTrackerState {
    fn new_empty(base: OLBlockCommitment) -> Self {
        Self {
            base,
            blocks: VecDeque::new(),
            data: HashMap::new(),
        }
    }

    pub(crate) fn best_block(&self) -> OLBlockCommitment {
        *self.blocks.back().unwrap_or(&self.base)
    }

    pub(crate) fn append_block(
        &mut self,
        block: OLBlockCommitment,
        inbox_messages: Vec<MessageEntry>,
    ) -> eyre::Result<()> {
        if block.slot() != self.best_block().slot() + 1 {
            return Err(eyre!("invalid block; block must extend existing chain"));
        }

        if self.data.contains_key(block.blkid()) {
            return Err(eyre!(
                "duplicate blkid: block {} already tracked",
                block.blkid()
            ));
        }

        self.blocks.push_back(block);
        self.data.insert(*block.blkid(), inbox_messages);

        Ok(())
    }

    pub(crate) fn prune_blocks(&mut self, next_base: OLBlockCommitment) -> eyre::Result<()> {
        if next_base == self.base {
            // noop
            return Ok(());
        }

        // binary_search requires sorted order. blocks is kept sorted by (slot, blkid)
        // since append_block enforces consecutive slots.
        let Ok(prune_idx) = self.blocks.binary_search(&next_base) else {
            // not a tracked block
            return Err(eyre!("unknown block: {next_base:?}"));
        };

        self.base = next_base;
        for _ in 0..=prune_idx {
            let block = self.blocks.pop_front().expect("should exist");
            self.data.remove(block.blkid());
        }

        Ok(())
    }

    pub(crate) fn get_inbox_messages(
        &self,
        mut from_slot: u64,
        mut to_slot: u64,
    ) -> eyre::Result<Vec<MessageEntry>> {
        if from_slot > to_slot {
            return Err(eyre!(
                "invalid query: from > to; from = {from_slot}, to = {to_slot}"
            ));
        }

        let (min_slot, max_slot) = match (self.blocks.front(), self.blocks.back()) {
            (Some(min_block), Some(max_block)) => (min_block.slot(), max_block.slot()),
            _ => {
                warn!("requested inbox messages from empty tracker");
                return Ok(vec![]);
            }
        };
        if from_slot < min_slot {
            warn!(
                min = min_slot,
                requested = from_slot,
                "requested inbox messages below min slot"
            );
            from_slot = min_slot;
        }
        if to_slot > max_slot {
            warn!(
                max = max_slot,
                requested = to_slot,
                "requested inbox messages above max slot"
            );
            to_slot = max_slot;
        }

        let valid_blocks = self
            .blocks
            .iter()
            .filter(|b| from_slot <= b.slot() && b.slot() <= to_slot);

        let mut res = Vec::new();
        for block in valid_blocks {
            let inbox_messages = self.data.get(block.blkid()).ok_or(eyre!(
                "missing inbox data for block ({}, {})",
                block.slot(),
                block.blkid()
            ))?;

            for message in inbox_messages {
                res.push(message.clone());
            }
        }

        Ok(res)
    }
}

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

    let mut state = OLChainTrackerState::new_empty(local_finalized_ol_block);

    // chain status according to OL
    // TODO: retry
    let ol_chain_status = ol_client.chain_status().await?;
    let remote_finalized_ol_block = ol_chain_status.finalized().to_block_commitment();

    if remote_finalized_ol_block == local_finalized_ol_block {
        // no new finalized blocks available to be processed.
        return Ok(state);
    }

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

    // Everything looks ok now. Build local state.
    for block in blocks {
        state.append_block(block.commitment, block.inbox_messages)?;
    }

    Ok(state)
}

#[cfg(test)]
mod tests {
    use strata_acct_types::{AccountId, BitcoinAmount, MsgPayload};
    use strata_identifiers::Buf32;

    use super::*;

    /// Helper to create a block commitment with a given slot.
    fn make_block(slot: u64) -> OLBlockCommitment {
        let mut blkid_bytes = [0u8; 32];
        blkid_bytes[0..8].copy_from_slice(&slot.to_le_bytes());
        OLBlockCommitment::new(slot, OLBlockId::from(Buf32::from(blkid_bytes)))
    }

    /// Helper to create a block commitment with a given slot and specific blkid bytes.
    fn make_block_with_id(slot: u64, id_byte: u8) -> OLBlockCommitment {
        let mut blkid_bytes = [id_byte; 32];
        blkid_bytes[0..8].copy_from_slice(&slot.to_le_bytes());
        OLBlockCommitment::new(slot, OLBlockId::from(Buf32::from(blkid_bytes)))
    }

    /// Helper to create a dummy message entry.
    fn make_message(value: u64) -> MessageEntry {
        MessageEntry::new(
            AccountId::new([0u8; 32]),
            0,
            MsgPayload::new(BitcoinAmount::from_sat(value), vec![]),
        )
    }

    mod best_block {
        use super::*;

        #[test]
        fn returns_base_when_empty() {
            let base = make_block(10);
            let state = OLChainTrackerState::new_empty(base);

            assert_eq!(state.best_block(), base);
        }

        #[test]
        fn returns_latest_appended_block() {
            let base = make_block(10);
            let mut state = OLChainTrackerState::new_empty(base);

            let block1 = make_block(11);
            let block2 = make_block(12);
            state.append_block(block1, vec![]).unwrap();
            state.append_block(block2, vec![]).unwrap();

            assert_eq!(state.best_block(), block2);
        }
    }

    mod append_block {
        use super::*;

        #[test]
        fn appends_consecutive_block() {
            let base = make_block(10);
            let mut state = OLChainTrackerState::new_empty(base);

            let block = make_block(11);
            let messages = vec![make_message(100)];
            state.append_block(block, messages.clone()).unwrap();

            assert_eq!(state.best_block(), block);
            assert_eq!(state.data.get(block.blkid()).unwrap().len(), 1);
        }

        #[test]
        fn appends_multiple_consecutive_blocks() {
            let base = make_block(10);
            let mut state = OLChainTrackerState::new_empty(base);

            for slot in 11..=15 {
                let block = make_block(slot);
                state.append_block(block, vec![]).unwrap();
            }

            assert_eq!(state.best_block().slot(), 15);
            assert_eq!(state.blocks.len(), 5);
        }

        #[test]
        fn rejects_non_consecutive_slot_gap() {
            let base = make_block(10);
            let mut state = OLChainTrackerState::new_empty(base);

            let block = make_block(12); // gap: skipped slot 11
            let result = state.append_block(block, vec![]);

            assert!(result.is_err());
            assert!(result
                .unwrap_err()
                .to_string()
                .contains("block must extend existing chain"));
        }

        #[test]
        fn rejects_slot_less_than_best() {
            let base = make_block(10);
            let mut state = OLChainTrackerState::new_empty(base);

            state.append_block(make_block(11), vec![]).unwrap();

            let block = make_block(10); // same as base
            let result = state.append_block(block, vec![]);

            assert!(result.is_err());
        }

        #[test]
        fn rejects_duplicate_blkid() {
            let base = make_block(10);
            let mut state = OLChainTrackerState::new_empty(base);

            let block1 = make_block(11);
            state.append_block(block1, vec![]).unwrap();

            // Create block with slot 12 but same blkid as block1
            let block2 = OLBlockCommitment::new(12, *block1.blkid());
            let result = state.append_block(block2, vec![]);

            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("duplicate blkid"));
        }

        #[test]
        fn accepts_empty_inbox_messages() {
            let base = make_block(10);
            let mut state = OLChainTrackerState::new_empty(base);

            let block = make_block(11);
            state.append_block(block, vec![]).unwrap();

            assert!(state.data.get(block.blkid()).unwrap().is_empty());
        }
    }

    mod prune_blocks {
        use super::*;

        #[test]
        fn noop_when_pruning_to_current_base() {
            let base = make_block(10);
            let mut state = OLChainTrackerState::new_empty(base);

            state.append_block(make_block(11), vec![]).unwrap();
            state.append_block(make_block(12), vec![]).unwrap();

            let result = state.prune_blocks(base);
            assert!(result.is_ok());
            assert_eq!(state.blocks.len(), 2);
        }

        #[test]
        fn prunes_to_tracked_block() {
            let base = make_block(10);
            let mut state = OLChainTrackerState::new_empty(base);

            let block11 = make_block(11);
            let block12 = make_block(12);
            let block13 = make_block(13);

            state.append_block(block11, vec![make_message(1)]).unwrap();
            state.append_block(block12, vec![make_message(2)]).unwrap();
            state.append_block(block13, vec![make_message(3)]).unwrap();

            state.prune_blocks(block12).unwrap();

            // block12 becomes new base, blocks 11 and 12 are removed
            assert_eq!(state.base, block12);
            assert_eq!(state.blocks.len(), 1);
            assert_eq!(state.blocks.front().unwrap().slot(), 13);
            // Data for pruned blocks should be removed
            assert!(!state.data.contains_key(block11.blkid()));
            assert!(!state.data.contains_key(block12.blkid()));
            assert!(state.data.contains_key(block13.blkid()));
        }

        #[test]
        fn prunes_to_first_tracked_block() {
            let base = make_block(10);
            let mut state = OLChainTrackerState::new_empty(base);

            let block11 = make_block(11);
            let block12 = make_block(12);

            state.append_block(block11, vec![]).unwrap();
            state.append_block(block12, vec![]).unwrap();

            state.prune_blocks(block11).unwrap();

            assert_eq!(state.base, block11);
            assert_eq!(state.blocks.len(), 1);
            assert_eq!(state.blocks.front().unwrap().slot(), 12);
        }

        #[test]
        fn prunes_to_last_tracked_block() {
            let base = make_block(10);
            let mut state = OLChainTrackerState::new_empty(base);

            let block11 = make_block(11);
            let block12 = make_block(12);

            state.append_block(block11, vec![]).unwrap();
            state.append_block(block12, vec![]).unwrap();

            state.prune_blocks(block12).unwrap();

            assert_eq!(state.base, block12);
            assert!(state.blocks.is_empty());
        }

        #[test]
        fn rejects_unknown_block() {
            let base = make_block(10);
            let mut state = OLChainTrackerState::new_empty(base);

            state.append_block(make_block(11), vec![]).unwrap();

            let unknown = make_block(15);
            let result = state.prune_blocks(unknown);

            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("unknown block"));
        }

        #[test]
        fn rejects_block_with_wrong_blkid_at_same_slot() {
            let base = make_block(10);
            let mut state = OLChainTrackerState::new_empty(base);

            let block11 = make_block_with_id(11, 0xAA);
            state.append_block(block11, vec![]).unwrap();

            // Same slot but different blkid
            let wrong_block = make_block_with_id(11, 0xBB);
            let result = state.prune_blocks(wrong_block);

            assert!(result.is_err());
        }
    }

    mod get_inbox_messages {
        use super::*;

        #[test]
        fn returns_empty_for_empty_tracker() {
            let base = make_block(10);
            let state = OLChainTrackerState::new_empty(base);

            let result = state.get_inbox_messages(10, 15).unwrap();
            assert!(result.is_empty());
        }

        #[test]
        fn returns_error_when_from_greater_than_to() {
            let base = make_block(10);
            let state = OLChainTrackerState::new_empty(base);

            let result = state.get_inbox_messages(15, 10);
            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("from > to"));
        }

        #[test]
        fn returns_messages_for_exact_range() {
            let base = make_block(10);
            let mut state = OLChainTrackerState::new_empty(base);

            state
                .append_block(make_block(11), vec![make_message(100)])
                .unwrap();
            state
                .append_block(make_block(12), vec![make_message(200)])
                .unwrap();
            state
                .append_block(make_block(13), vec![make_message(300)])
                .unwrap();

            let messages = state.get_inbox_messages(11, 13).unwrap();
            assert_eq!(messages.len(), 3);
            assert_eq!(messages[0].payload_value().to_sat(), 100);
            assert_eq!(messages[1].payload_value().to_sat(), 200);
            assert_eq!(messages[2].payload_value().to_sat(), 300);
        }

        #[test]
        fn returns_messages_for_partial_range() {
            let base = make_block(10);
            let mut state = OLChainTrackerState::new_empty(base);

            state
                .append_block(make_block(11), vec![make_message(100)])
                .unwrap();
            state
                .append_block(make_block(12), vec![make_message(200)])
                .unwrap();
            state
                .append_block(make_block(13), vec![make_message(300)])
                .unwrap();

            let messages = state.get_inbox_messages(12, 12).unwrap();
            assert_eq!(messages.len(), 1);
            assert_eq!(messages[0].payload_value().to_sat(), 200);
        }

        #[test]
        fn clamps_from_slot_to_min() {
            let base = make_block(10);
            let mut state = OLChainTrackerState::new_empty(base);

            state
                .append_block(make_block(11), vec![make_message(100)])
                .unwrap();
            state
                .append_block(make_block(12), vec![make_message(200)])
                .unwrap();

            // Request from slot 5, but min is 11
            let messages = state.get_inbox_messages(5, 12).unwrap();
            assert_eq!(messages.len(), 2);
        }

        #[test]
        fn clamps_to_slot_to_max() {
            let base = make_block(10);
            let mut state = OLChainTrackerState::new_empty(base);

            state
                .append_block(make_block(11), vec![make_message(100)])
                .unwrap();
            state
                .append_block(make_block(12), vec![make_message(200)])
                .unwrap();

            // Request to slot 20, but max is 12
            let messages = state.get_inbox_messages(11, 20).unwrap();
            assert_eq!(messages.len(), 2);
        }

        #[test]
        fn returns_empty_when_range_outside_tracked() {
            let base = make_block(10);
            let mut state = OLChainTrackerState::new_empty(base);

            state
                .append_block(make_block(11), vec![make_message(100)])
                .unwrap();
            state
                .append_block(make_block(12), vec![make_message(200)])
                .unwrap();

            // Request range completely below tracked blocks (after clamping from<to check)
            // from=20 gets clamped to min=11, to=25 gets clamped to max=12
            // This actually returns messages since clamping brings it into range
            let messages = state.get_inbox_messages(20, 25).unwrap();
            // After clamping: from=11, to=12 (since 20>12 clamps to 12, 25>12 clamps to 12)
            // Actually from=20 < min=11 is false, so no clamping on from
            // Wait, 20 > 11 so from_slot < min_slot is false
            // to=25 > max=12 so to_slot gets clamped to 12
            // Final range: from=20, to=12 ... but 20 > 12, so filter returns nothing
            assert!(messages.is_empty());
        }

        #[test]
        fn handles_multiple_messages_per_block() {
            let base = make_block(10);
            let mut state = OLChainTrackerState::new_empty(base);

            state
                .append_block(
                    make_block(11),
                    vec![make_message(100), make_message(101), make_message(102)],
                )
                .unwrap();

            let messages = state.get_inbox_messages(11, 11).unwrap();
            assert_eq!(messages.len(), 3);
        }
    }

    mod init_ol_chain_tracker_state_tests {
        use alpen_ee_common::{
            MockExecBlockStorage, MockSequencerOLClient, OLBlockData, OLChainStatus, OLClientError,
        };
        use strata_identifiers::EpochCommitment;

        use super::*;

        // =========================================================================
        // Test Helpers
        // =========================================================================

        /// Helper to create an OLBlockCommitment with a given slot and id byte.
        /// The id_byte is placed at the start of the blkid for easy identification.
        fn make_ol_block(slot: u64, id_byte: u8) -> OLBlockCommitment {
            let mut blkid_bytes = [0u8; 32];
            blkid_bytes[0] = id_byte;
            blkid_bytes[1] = 1; // Ensure non-null
            OLBlockCommitment::new(slot, OLBlockId::from(Buf32::from(blkid_bytes)))
        }

        /// Helper to create an EpochCommitment from an OLBlockCommitment.
        fn make_epoch_from_block(epoch: u32, block: OLBlockCommitment) -> EpochCommitment {
            EpochCommitment::new(epoch, block.slot(), *block.blkid())
        }

        /// Helper to create OLChainStatus with the given finalized block.
        fn make_chain_status(finalized: OLBlockCommitment) -> OLChainStatus {
            let epoch = make_epoch_from_block(0, finalized);
            OLChainStatus {
                latest: finalized,
                confirmed: epoch,
                finalized: epoch,
            }
        }

        /// Helper to create OLBlockData for a block.
        fn make_block_data(block: OLBlockCommitment, messages: Vec<MessageEntry>) -> OLBlockData {
            OLBlockData {
                commitment: block,
                inbox_messages: messages,
            }
        }

        /// Creates a chain of OL blocks starting from base_slot.
        ///
        /// Returns blocks with slots [base_slot, base_slot+1, ..., base_slot+count-1]
        /// Each block has id_byte = slot as u8 for easy identification.
        ///
        /// # Example
        /// ```
        /// // Creates blocks at slots 10, 11, 12
        /// let blocks = create_ol_block_chain(10, 3);
        /// assert_eq!(blocks[0].slot(), 10);
        /// assert_eq!(blocks[2].slot(), 12);
        /// ```
        fn create_ol_block_chain(base_slot: u64, count: usize) -> Vec<OLBlockCommitment> {
            (0..count)
                .map(|i| {
                    let slot = base_slot + i as u64;
                    make_ol_block(slot, slot as u8)
                })
                .collect()
        }

        /// Creates OLBlockData for each block in the chain.
        /// Each block gets one message with value = slot * 100.
        fn create_block_data_chain(blocks: &[OLBlockCommitment]) -> Vec<OLBlockData> {
            blocks
                .iter()
                .map(|block| {
                    let msg = make_message(block.slot() * 100);
                    make_block_data(*block, vec![msg])
                })
                .collect()
        }

        /// Creates a mock ExecBlockRecord that references the given OL block.
        /// Uses the test helper from alpen_ee_common.
        fn create_mock_exec_record(
            ol_block: OLBlockCommitment,
        ) -> alpen_ee_common::ExecBlockRecord {
            use strata_acct_types::Hash;
            use strata_ee_acct_types::EeAccountState;
            use strata_ee_chain_types::{
                BlockInputs, BlockOutputs, ExecBlockCommitment, ExecBlockPackage,
            };
            use strata_identifiers::Buf32;

            let hash_bytes = [ol_block.slot() as u8; 32];
            let hash = Hash::from(Buf32::new(hash_bytes));

            let package = ExecBlockPackage::new(
                ExecBlockCommitment::new(hash, hash),
                BlockInputs::new_empty(),
                BlockOutputs::new_empty(),
            );

            let account_state =
                EeAccountState::new(hash, strata_acct_types::BitcoinAmount::ZERO, vec![], vec![]);

            alpen_ee_common::ExecBlockRecord::new(
                package,
                account_state,
                ol_block.slot(),
                ol_block,
                1_000_000,
                Hash::default(),
            )
        }

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
            block_data: Vec<OLBlockData>,
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

            let finalized_block = make_ol_block(10, 10);
            let exec_record = create_mock_exec_record(finalized_block);
            let chain_status = make_chain_status(finalized_block);

            let mut mock_storage = MockExecBlockStorage::new();
            let mut mock_client = MockSequencerOLClient::new();

            setup_mock_storage_finalized(&mut mock_storage, exec_record);
            setup_mock_client_chain_status(&mut mock_client, chain_status);

            let state = init_ol_chain_tracker_state(&mock_storage, &mock_client)
                .await
                .unwrap();

            assert_eq!(state.best_block(), finalized_block);
            assert!(state.blocks.is_empty());
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

            let local_finalized = make_ol_block(10, 10);
            let remote_finalized = make_ol_block(13, 13);

            // Create block chain from slot 10 to 13
            let ol_blocks = create_ol_block_chain(10, 4); // slots 10, 11, 12, 13
            let block_data = create_block_data_chain(&ol_blocks);

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
            assert_eq!(state.base.slot(), 10);
            // Should have 3 new blocks tracked (11, 12, 13)
            assert_eq!(state.blocks.len(), 3);
            assert_eq!(state.best_block().slot(), 13);

            // Verify messages were stored
            let messages = state.get_inbox_messages(11, 13).unwrap();
            assert_eq!(messages.len(), 3);
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

            let local_finalized = make_ol_block(15, 15);
            let remote_finalized = make_ol_block(10, 10);

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

            let local_finalized = make_ol_block(10, 0xAA);
            let remote_finalized = make_ol_block(11, 11);

            // Remote returns different block at slot 10
            let remote_block_at_10 = make_ol_block(10, 0xBB);
            let remote_block_at_11 = make_ol_block(11, 11);
            let block_data = vec![
                make_block_data(remote_block_at_10, vec![]),
                make_block_data(remote_block_at_11, vec![]),
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

            let local_finalized = make_ol_block(10, 10);
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

            let local_finalized = make_ol_block(10, 10);
            let remote_finalized = make_ol_block(11, 11);

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
