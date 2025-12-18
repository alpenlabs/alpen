use std::collections::{HashMap, VecDeque};

use eyre::eyre;
use strata_identifiers::{OLBlockCommitment, OLBlockId};
use strata_snark_acct_types::MessageEntry;
use tracing::warn;

#[derive(Debug)]
pub(crate) struct BlockInfo {
    pub messages: Vec<MessageEntry>,
    pub next_inbox_msg_idx: u64,
}

impl BlockInfo {
    fn new(messages: Vec<MessageEntry>, next_inbox_msg_idx: u64) -> Self {
        Self {
            messages,
            next_inbox_msg_idx,
        }
    }
}

#[derive(Debug)]
pub struct InboxMessages {
    pub messages: Vec<MessageEntry>,
    pub next_inbox_msg_idx: u64,
}

impl InboxMessages {
    pub fn empty(next_inbox_msg_idx: u64) -> Self {
        Self {
            messages: vec![],
            next_inbox_msg_idx,
        }
    }
}

/// Tracks OL chain blocks and their inbox messages for the sequencer.
#[derive(Debug)]
pub struct OLChainTrackerState {
    /// Lowest block being tracked.
    /// The messages upto this block have already been processed.
    base_block: OLBlockCommitment,
    /// next_inbox_msg_idx corresponding to `base_block`
    base_block_next_inbox_msg_idx: u64,
    /// blocks whose messages have not been processed.
    blocks: VecDeque<OLBlockCommitment>,
    /// messages in the blocks.
    data: HashMap<OLBlockId, BlockInfo>,
}

#[cfg(test)]
#[allow(unused, clippy::allow_attributes, reason = "test accessors")]
impl OLChainTrackerState {
    pub(crate) fn base(&self) -> &OLBlockCommitment {
        &self.base_block
    }

    pub(crate) fn blocks(&self) -> &VecDeque<OLBlockCommitment> {
        &self.blocks
    }

    pub(crate) fn data(&self) -> &HashMap<OLBlockId, BlockInfo> {
        &self.data
    }
}

impl OLChainTrackerState {
    pub(crate) fn new_empty(
        base_block: OLBlockCommitment,
        base_block_next_inbox_msg_idx: u64,
    ) -> Self {
        Self {
            base_block,
            base_block_next_inbox_msg_idx,
            blocks: VecDeque::new(),
            data: HashMap::new(),
        }
    }

    /// Returns the most recent tracked block, or the base if no blocks are tracked.
    pub(crate) fn best_block(&self) -> OLBlockCommitment {
        *self.blocks.back().unwrap_or(&self.base_block)
    }

    /// Appends a block and its inbox messages. The block must extend the current chain.
    pub(crate) fn append_block(
        &mut self,
        block: OLBlockCommitment,
        inbox_messages: Vec<MessageEntry>,
        next_inbox_msg_idx: u64,
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
        self.data.insert(
            *block.blkid(),
            BlockInfo::new(inbox_messages, next_inbox_msg_idx),
        );

        Ok(())
    }

    /// Prunes blocks up to and including `next_base`, which becomes the new base.
    pub(crate) fn prune_blocks(&mut self, next_base: OLBlockCommitment) -> eyre::Result<()> {
        if next_base == self.base_block {
            // noop
            return Ok(());
        }

        // binary_search requires sorted order. blocks is kept sorted by (slot, blkid)
        // since append_block enforces consecutive slots.
        let Ok(prune_idx) = self.blocks.binary_search(&next_base) else {
            // not a tracked block
            return Err(eyre!("unknown block: {next_base:?}"));
        };

        self.base_block = next_base;
        for _ in 0..=prune_idx {
            let block = self.blocks.pop_front().expect("should exist");
            self.data.remove(block.blkid());
        }

        Ok(())
    }

    /// Returns inbox messages for blocks in the given slot range (inclusive).
    pub(crate) fn get_inbox_messages(
        &self,
        mut from_slot: u64,
        mut to_slot: u64,
    ) -> eyre::Result<InboxMessages> {
        if from_slot > to_slot {
            return Err(eyre!(
                "invalid query: from > to; from = {from_slot}, to = {to_slot}"
            ));
        }

        let (min_slot, max_slot) = match (self.blocks.front(), self.blocks.back()) {
            (Some(min_block), Some(max_block)) => (min_block.slot(), max_block.slot()),
            _ => {
                warn!("requested inbox messages from empty tracker");
                return Ok(InboxMessages {
                    messages: vec![],
                    next_inbox_msg_idx: self.base_block_next_inbox_msg_idx,
                });
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

        // Blocks are present
        // Index of first block with slot >= from_slot
        let from_idx = self.blocks.partition_point(|b| b.slot() < from_slot);

        // Get next_inbox_msg_idx from the block before from_idx, or use base
        let next_inbox_msg_idx = from_idx
            .checked_sub(1)
            .and_then(|i| self.blocks.get(i))
            .and_then(|b| self.data.get(b.blkid()))
            .map_or(self.base_block_next_inbox_msg_idx, |d| d.next_inbox_msg_idx);

        let messages = self
            .blocks
            .iter()
            .skip(from_idx)
            .take_while(|b| b.slot() <= to_slot)
            .map(|b| {
                self.data
                    .get(b.blkid())
                    .ok_or_else(|| {
                        eyre!("missing inbox data for block ({}, {})", b.slot(), b.blkid())
                    })
                    .map(|d| d.messages.clone())
            })
            .collect::<eyre::Result<Vec<_>>>()?
            .into_iter()
            .flatten()
            .collect();

        Ok(InboxMessages {
            messages,
            next_inbox_msg_idx,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ol_chain_tracker::test_utils::{make_block, make_block_with_id, make_message};

    mod best_block {
        use super::*;

        #[test]
        fn returns_base_when_empty() {
            let base = make_block(10);
            let state = OLChainTrackerState::new_empty(base, 0);

            assert_eq!(state.best_block(), base);
        }

        #[test]
        fn returns_latest_appended_block() {
            let base = make_block(10);
            let mut state = OLChainTrackerState::new_empty(base, 0);

            let block1 = make_block(11);
            let block2 = make_block(12);
            state.append_block(block1, vec![], 0).unwrap();
            state.append_block(block2, vec![], 0).unwrap();

            assert_eq!(state.best_block(), block2);
        }
    }

    mod append_block {
        use super::*;

        #[test]
        fn appends_consecutive_block() {
            let base = make_block(10);
            let mut state = OLChainTrackerState::new_empty(base, 0);

            let block = make_block(11);
            let messages = vec![make_message(100)];
            state.append_block(block, messages.clone(), 0).unwrap();

            assert_eq!(state.best_block(), block);
            assert_eq!(state.data.get(block.blkid()).unwrap().messages.len(), 1);
        }

        #[test]
        fn appends_multiple_consecutive_blocks() {
            let base = make_block(10);
            let mut state = OLChainTrackerState::new_empty(base, 0);

            for slot in 11..=15 {
                let block = make_block(slot);
                state.append_block(block, vec![], 0).unwrap();
            }

            assert_eq!(state.best_block().slot(), 15);
            assert_eq!(state.blocks.len(), 5);
        }

        #[test]
        fn rejects_non_consecutive_slot_gap() {
            let base = make_block(10);
            let mut state = OLChainTrackerState::new_empty(base, 0);

            let block = make_block(12); // gap: skipped slot 11
            let result = state.append_block(block, vec![], 0);

            assert!(result.is_err());
            assert!(result
                .unwrap_err()
                .to_string()
                .contains("block must extend existing chain"));
        }

        #[test]
        fn rejects_slot_less_than_best() {
            let base = make_block(10);
            let mut state = OLChainTrackerState::new_empty(base, 0);

            state.append_block(make_block(11), vec![], 0).unwrap();

            let block = make_block(10); // same as base
            let result = state.append_block(block, vec![], 0);

            assert!(result.is_err());
        }

        #[test]
        fn rejects_duplicate_blkid() {
            let base = make_block(10);
            let mut state = OLChainTrackerState::new_empty(base, 0);

            let block1 = make_block(11);
            state.append_block(block1, vec![], 0).unwrap();

            // Create block with slot 12 but same blkid as block1
            let block2 = OLBlockCommitment::new(12, *block1.blkid());
            let result = state.append_block(block2, vec![], 0);

            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("duplicate blkid"));
        }

        #[test]
        fn accepts_empty_inbox_messages() {
            let base = make_block(10);
            let mut state = OLChainTrackerState::new_empty(base, 0);

            let block = make_block(11);
            state.append_block(block, vec![], 0).unwrap();

            assert!(state.data.get(block.blkid()).unwrap().messages.is_empty());
        }
    }

    mod prune_blocks {
        use super::*;

        #[test]
        fn noop_when_pruning_to_current_base() {
            let base = make_block(10);
            let mut state = OLChainTrackerState::new_empty(base, 0);

            state.append_block(make_block(11), vec![], 0).unwrap();
            state.append_block(make_block(12), vec![], 0).unwrap();

            let result = state.prune_blocks(base);
            assert!(result.is_ok());
            assert_eq!(state.blocks.len(), 2);
        }

        #[test]
        fn prunes_to_tracked_block() {
            let base = make_block(10);
            let mut state = OLChainTrackerState::new_empty(base, 0);

            let block11 = make_block(11);
            let block12 = make_block(12);
            let block13 = make_block(13);

            state
                .append_block(block11, vec![make_message(1)], 0)
                .unwrap();
            state
                .append_block(block12, vec![make_message(2)], 0)
                .unwrap();
            state
                .append_block(block13, vec![make_message(3)], 0)
                .unwrap();

            state.prune_blocks(block12).unwrap();

            // block12 becomes new base, blocks 11 and 12 are removed
            assert_eq!(state.base_block, block12);
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
            let mut state = OLChainTrackerState::new_empty(base, 0);

            let block11 = make_block(11);
            let block12 = make_block(12);

            state.append_block(block11, vec![], 0).unwrap();
            state.append_block(block12, vec![], 0).unwrap();

            state.prune_blocks(block11).unwrap();

            assert_eq!(state.base_block, block11);
            assert_eq!(state.blocks.len(), 1);
            assert_eq!(state.blocks.front().unwrap().slot(), 12);
        }

        #[test]
        fn prunes_to_last_tracked_block() {
            let base = make_block(10);
            let mut state = OLChainTrackerState::new_empty(base, 0);

            let block11 = make_block(11);
            let block12 = make_block(12);

            state.append_block(block11, vec![], 0).unwrap();
            state.append_block(block12, vec![], 0).unwrap();

            state.prune_blocks(block12).unwrap();

            assert_eq!(state.base_block, block12);
            assert!(state.blocks.is_empty());
        }

        #[test]
        fn rejects_unknown_block() {
            let base = make_block(10);
            let mut state = OLChainTrackerState::new_empty(base, 0);

            state.append_block(make_block(11), vec![], 0).unwrap();

            let unknown = make_block(15);
            let result = state.prune_blocks(unknown);

            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("unknown block"));
        }

        #[test]
        fn rejects_block_with_wrong_blkid_at_same_slot() {
            let base = make_block(10);
            let mut state = OLChainTrackerState::new_empty(base, 0);

            let block11 = make_block_with_id(11, 0xAA);
            state.append_block(block11, vec![], 0).unwrap();

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
            let state = OLChainTrackerState::new_empty(base, 0);

            let result = state.get_inbox_messages(10, 15).unwrap();
            assert!(result.messages.is_empty());
        }

        #[test]
        fn returns_error_when_from_greater_than_to() {
            let base = make_block(10);
            let state = OLChainTrackerState::new_empty(base, 0);

            let result = state.get_inbox_messages(15, 10);
            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("from > to"));
        }

        #[test]
        fn returns_messages_for_exact_range() {
            let base = make_block(10);
            let mut state = OLChainTrackerState::new_empty(base, 0);

            state
                .append_block(make_block(11), vec![make_message(100)], 0)
                .unwrap();
            state
                .append_block(make_block(12), vec![make_message(200)], 1)
                .unwrap();
            state
                .append_block(make_block(13), vec![make_message(300)], 2)
                .unwrap();

            let messages = state.get_inbox_messages(11, 13).unwrap();
            assert_eq!(messages.messages.len(), 3);
            assert_eq!(messages.messages[0].payload_value().to_sat(), 100);
            assert_eq!(messages.messages[1].payload_value().to_sat(), 200);
            assert_eq!(messages.messages[2].payload_value().to_sat(), 300);
        }

        #[test]
        fn returns_messages_for_partial_range() {
            let base = make_block(10);
            let mut state = OLChainTrackerState::new_empty(base, 0);

            state
                .append_block(make_block(11), vec![make_message(100)], 0)
                .unwrap();
            state
                .append_block(make_block(12), vec![make_message(200)], 0)
                .unwrap();
            state
                .append_block(make_block(13), vec![make_message(300)], 0)
                .unwrap();

            let messages = state.get_inbox_messages(12, 12).unwrap();
            assert_eq!(messages.messages.len(), 1);
            assert_eq!(messages.messages[0].payload_value().to_sat(), 200);
        }

        #[test]
        fn clamps_from_slot_to_min() {
            let base = make_block(10);
            let mut state = OLChainTrackerState::new_empty(base, 0);

            state
                .append_block(make_block(11), vec![make_message(100)], 1)
                .unwrap();
            state
                .append_block(make_block(12), vec![make_message(200)], 2)
                .unwrap();

            // Request from slot 5, but min is 11
            let messages = state.get_inbox_messages(5, 12).unwrap();
            assert_eq!(messages.messages.len(), 2);
        }

        #[test]
        fn clamps_to_slot_to_max() {
            let base = make_block(10);
            let mut state = OLChainTrackerState::new_empty(base, 0);

            state
                .append_block(make_block(11), vec![make_message(100)], 1)
                .unwrap();
            state
                .append_block(make_block(12), vec![make_message(200)], 2)
                .unwrap();

            // Request to slot 20, but max is 12
            let messages = state.get_inbox_messages(11, 20).unwrap();
            assert_eq!(messages.messages.len(), 2);
        }

        #[test]
        fn returns_empty_when_range_outside_tracked() {
            let base = make_block(10);
            let mut state = OLChainTrackerState::new_empty(base, 0);

            state
                .append_block(make_block(11), vec![make_message(100)], 1)
                .unwrap();
            state
                .append_block(make_block(12), vec![make_message(200)], 2)
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
            assert!(messages.messages.is_empty());
        }

        #[test]
        fn handles_multiple_messages_per_block() {
            let base = make_block(10);
            let mut state = OLChainTrackerState::new_empty(base, 0);

            state
                .append_block(
                    make_block(11),
                    vec![make_message(100), make_message(101), make_message(102)],
                    3,
                )
                .unwrap();

            let messages = state.get_inbox_messages(11, 11).unwrap();
            assert_eq!(messages.messages.len(), 3);
        }
    }
}
