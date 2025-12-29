use std::collections::HashMap;

use alpen_ee_common::ExecBlockRecord;
use strata_acct_types::Hash;
use thiserror::Error;

/// Errors that can occur in the unfinalized tracker.
#[derive(Debug, Error)]
pub enum UnfinalizedTrackerError {
    /// Block not found in tracker
    #[error("unknown block: {0:?}")]
    UnknownBlock(Hash),
    /// Invalid tracker state
    #[error("invalid tracker state")]
    InvalidState,
}

/// A block identifier combining hash and height.
#[derive(Debug, Clone, Copy)]
pub(crate) struct BlockNumHash {
    pub hash: Hash,
    pub height: u64,
}

/// Block metadata needed for chain tracking.
#[derive(Debug, Clone)]
pub(crate) struct BlockEntry {
    pub blocknum: u64,
    pub blockhash: Hash,
    pub parent: Hash,
}

impl From<&ExecBlockRecord> for BlockEntry {
    fn from(value: &ExecBlockRecord) -> Self {
        Self {
            blockhash: value.blockhash(),
            blocknum: value.blocknum(),
            parent: value.parent_blockhash(),
        }
    }
}

/// Tracks unfinalized blocks and maintains chain tips between the finalized block and the best tip.
///
/// Manages a tree of blocks starting from the last finalized block, tracking all competing
/// chain tips and identifying the best (highest) tip.
#[derive(Debug)]
pub(crate) struct UnfinalizedTracker {
    /// The last finalized block
    finalized: BlockNumHash,
    /// The current best (highest) chain tip
    best: BlockNumHash,
    /// Active chain tips mapping hash to height
    tips: HashMap<Hash, u64>,
    /// All tracked blocks mapping hash to block entry
    blocks: HashMap<Hash, BlockEntry>,
}

/// Possible results of attaching block to [`UnfinalizedTracker`].
pub(crate) enum AttachBlockRes {
    /// Attached successfully.
    Ok(Hash),
    /// Block already exists.
    ExistingBlock,
    /// Block is below finalized height, cannot be attached.
    BelowFinalized(BlockEntry),
    /// Block does not extend any existing tip, cannot be attached.
    OrphanBlock(BlockEntry),
}

impl UnfinalizedTracker {
    /// Creates a new tracker with the given finalized block as the initial state.
    pub(crate) fn new_empty(finalized_block: BlockEntry) -> Self {
        let hash = finalized_block.blockhash;
        let height = finalized_block.blocknum;
        Self {
            finalized: BlockNumHash { hash, height },
            best: BlockNumHash { hash, height },
            tips: HashMap::from([(hash, height)]),
            blocks: HashMap::from([(hash, finalized_block)]),
        }
    }

    /// Attempts to attach a block to the tracker.
    ///
    /// Returns the result of the attachment attempt, updating tips and best block if successful.
    pub(crate) fn attach_block(&mut self, block: BlockEntry) -> AttachBlockRes {
        // 1. Is it an existing block ?
        let blockhash = block.blockhash;
        if self.blocks.contains_key(&blockhash) {
            return AttachBlockRes::ExistingBlock;
        }

        // 2. Is it below finalized ?
        let block_height = block.blocknum;
        if block_height < self.finalized.height {
            return AttachBlockRes::BelowFinalized(block);
        }

        // 3. Does it extend an existing tip ?
        let parent_blockhash = block.parent;
        if self.tips.contains_key(&parent_blockhash) {
            self.blocks.insert(blockhash, block);
            self.tips.remove(&parent_blockhash);
            self.tips.insert(blockhash, block_height);

            (self.best.hash, self.best.height) = self.compute_best_tip();
            return AttachBlockRes::Ok(self.best.hash);
        };

        // 4. does it create a new tip ?
        if self.blocks.contains_key(&parent_blockhash) {
            self.blocks.insert(blockhash, block);
            self.tips.insert(blockhash, block_height);

            (self.best.hash, self.best.height) = self.compute_best_tip();
            return AttachBlockRes::Ok(self.best.hash);
        }

        // does not extend any known block
        AttachBlockRes::OrphanBlock(block)
    }

    /// Finds the tip with the highest block height.
    fn compute_best_tip(&self) -> (Hash, u64) {
        let (hash, height) = self.tips.iter().fold(
            (&self.best.hash, &self.best.height),
            |(a_hash, a_height), (b_hash, b_height)| {
                if b_height > a_height {
                    (b_hash, b_height)
                } else {
                    (a_hash, a_height)
                }
            },
        );
        (*hash, *height)
    }

    /// Checks if a block with the given hash is tracked.
    pub(crate) fn contains_block(&self, hash: &Hash) -> bool {
        self.blocks.contains_key(hash)
    }

    /// Returns the current finalized block.
    pub(crate) fn finalized(&self) -> BlockNumHash {
        self.finalized
    }

    /// Returns the current best (highest) chain tip.
    pub(crate) fn best(&self) -> BlockNumHash {
        self.best
    }

    /// Checks if a block is on the canonical chain (the path from finalized to best).
    ///
    /// Returns `true` if the block is either the finalized block, on the path from
    /// the best tip back to the finalized block, or is the best tip itself.
    pub(crate) fn is_canonical(&self, hash: &Hash) -> bool {
        // Check if it's the finalized block
        if *hash == self.finalized.hash {
            return true;
        }

        // Check if the block exists at all
        if !self.blocks.contains_key(hash) {
            return false;
        }

        // Walk backwards from best tip to finalized, checking if we encounter the hash
        let mut current = self.best.hash;
        while current != self.finalized.hash {
            if current == *hash {
                return true;
            }
            // Get the parent of current block
            let Some(block) = self.blocks.get(&current) else {
                // Should not happen in a well-formed tracker
                return false;
            };
            current = block.parent;
        }

        false
    }

    /// Advances the finalized block and prunes the tracker, removing blocks not on the finalized
    /// chain.
    ///
    /// Returns a report of newly finalized blocks and blocks that were pruned.
    pub(crate) fn prune_finalized(
        &mut self,
        new_finalized: Hash,
    ) -> Result<FinalizeReport, UnfinalizedTrackerError> {
        if new_finalized == self.finalized.hash {
            // noop
            return Ok(FinalizeReport::new_empty());
        }

        let Some(new_finalized_block) = self.blocks.remove(&new_finalized) else {
            // unknown block
            return Err(UnfinalizedTrackerError::UnknownBlock(new_finalized));
        };

        // get all blocks that are newly finalized
        let finalized_blocks_count = new_finalized_block.blocknum - self.finalized.height;
        let mut finalized_hashes = Vec::<Hash>::with_capacity(finalized_blocks_count as usize);
        let mut block = new_finalized_block.clone();
        for _ in 0..finalized_blocks_count {
            finalized_hashes.push(block.blockhash);
            block = self.blocks.remove(&block.parent).expect("should exist");
        }

        // sanity check
        if block.blockhash != self.finalized.hash {
            return Err(UnfinalizedTrackerError::InvalidState);
        }

        // easier to just recreate the tracker using existing blocks
        let mut tmp_tracker = Self::new_empty(new_finalized_block);
        let mut blocks = self.blocks.drain().collect::<Vec<_>>();

        blocks.sort_by_cached_key(|(_, block)| block.blocknum);
        let mut removed = Vec::new();

        for (_, block) in blocks {
            match tmp_tracker.attach_block(block) {
                AttachBlockRes::OrphanBlock(block) => {
                    removed.push(block.blockhash);
                }
                AttachBlockRes::BelowFinalized(block) => {
                    removed.push(block.blockhash);
                }
                AttachBlockRes::Ok(_) => {}
                _ => unreachable!(),
            }
        }

        *self = tmp_tracker;

        finalized_hashes.reverse();
        Ok(FinalizeReport::new(finalized_hashes, removed))
    }
}

/// Report of blocks affected by finalization, used by caller to update their state.
#[derive(Debug)]
pub(crate) struct FinalizeReport {
    /// Blocks that became newly finalized (removed from tracker, now in canonical chain)
    pub(crate) finalize: Vec<Hash>,
    /// Blocks that no longer extend the finalized block and should be removed
    pub(crate) remove: Vec<Hash>,
}

impl FinalizeReport {
    /// Creates a new report with the given newly finalized and removed blocks.
    fn new(finalize: Vec<Hash>, remove: Vec<Hash>) -> Self {
        Self { finalize, remove }
    }

    /// Creates an empty report indicating no changes.
    fn new_empty() -> Self {
        Self {
            finalize: Vec::new(),
            remove: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use strata_identifiers::Buf32;

    use super::*;

    fn hash_from_u8(value: u8) -> Hash {
        Hash::from(Buf32::new([value; 32]))
    }

    fn make_block(blocknum: u64, blockhash: Hash, parent: Hash) -> BlockEntry {
        BlockEntry {
            blocknum,
            blockhash,
            parent,
        }
    }

    #[test]
    fn test_attach_block_to_finalized() {
        let finalized = make_block(0, hash_from_u8(0), hash_from_u8(0));
        let mut tracker = UnfinalizedTracker::new_empty(finalized);

        let block1 = make_block(1, hash_from_u8(1), hash_from_u8(0));
        let result = tracker.attach_block(block1);

        assert!(matches!(result, AttachBlockRes::Ok(_)));
        assert_eq!(tracker.best().hash, hash_from_u8(1));
        assert!(tracker.contains_block(&hash_from_u8(1)));
    }

    #[test]
    fn test_attach_linear_chain() {
        let finalized = make_block(0, hash_from_u8(0), hash_from_u8(0));
        let mut tracker = UnfinalizedTracker::new_empty(finalized);

        let block1 = make_block(1, hash_from_u8(1), hash_from_u8(0));
        let block2 = make_block(2, hash_from_u8(2), hash_from_u8(1));
        let block3 = make_block(3, hash_from_u8(3), hash_from_u8(2));

        tracker.attach_block(block1);
        tracker.attach_block(block2);
        tracker.attach_block(block3);

        assert_eq!(tracker.best().hash, hash_from_u8(3));
        assert_eq!(tracker.best().height, 3);
    }

    #[test]
    fn test_attach_fork() {
        //     0 (finalized)
        //    / \
        //   1   2
        //   |
        //   3
        let finalized = make_block(0, hash_from_u8(0), hash_from_u8(0));
        let mut tracker = UnfinalizedTracker::new_empty(finalized);

        let block1 = make_block(1, hash_from_u8(1), hash_from_u8(0));
        let block2 = make_block(1, hash_from_u8(2), hash_from_u8(0));
        let block3 = make_block(2, hash_from_u8(3), hash_from_u8(1));

        tracker.attach_block(block1);
        tracker.attach_block(block2);
        tracker.attach_block(block3);

        // Block 3 is tallest, so it should be best
        assert_eq!(tracker.best().hash, hash_from_u8(3));
        assert_eq!(tracker.best().height, 2);
        assert!(tracker.contains_block(&hash_from_u8(1)));
        assert!(tracker.contains_block(&hash_from_u8(2)));
        assert!(tracker.contains_block(&hash_from_u8(3)));
    }

    #[test]
    fn test_existing_block() {
        let finalized = make_block(0, hash_from_u8(0), hash_from_u8(0));
        let mut tracker = UnfinalizedTracker::new_empty(finalized);

        let block1 = make_block(1, hash_from_u8(1), hash_from_u8(0));
        tracker.attach_block(block1.clone());

        let result = tracker.attach_block(block1);
        assert!(matches!(result, AttachBlockRes::ExistingBlock));
    }

    #[test]
    fn test_below_finalized() {
        let finalized = make_block(5, hash_from_u8(5), hash_from_u8(4));
        let mut tracker = UnfinalizedTracker::new_empty(finalized);

        let block = make_block(3, hash_from_u8(3), hash_from_u8(2));
        let result = tracker.attach_block(block);

        assert!(matches!(result, AttachBlockRes::BelowFinalized(_)));
    }

    #[test]
    fn test_orphan_block() {
        let finalized = make_block(0, hash_from_u8(0), hash_from_u8(0));
        let mut tracker = UnfinalizedTracker::new_empty(finalized);

        // Try to attach block 2 without block 1
        let block2 = make_block(2, hash_from_u8(2), hash_from_u8(1));
        let result = tracker.attach_block(block2);

        assert!(matches!(result, AttachBlockRes::OrphanBlock(_)));
    }

    #[test]
    fn test_best_tip_selection() {
        //     0 (finalized)
        //    /|\
        //   1 2 3
        //     |
        //     4
        let finalized = make_block(0, hash_from_u8(0), hash_from_u8(0));
        let mut tracker = UnfinalizedTracker::new_empty(finalized);

        tracker.attach_block(make_block(1, hash_from_u8(1), hash_from_u8(0)));
        tracker.attach_block(make_block(1, hash_from_u8(2), hash_from_u8(0)));
        tracker.attach_block(make_block(1, hash_from_u8(3), hash_from_u8(0)));

        // All at height 1, best should be one of them
        assert_eq!(tracker.best().height, 1);

        // Add block 4 extending block 2
        tracker.attach_block(make_block(2, hash_from_u8(4), hash_from_u8(2)));

        // Now block 4 should be best (height 2)
        assert_eq!(tracker.best().hash, hash_from_u8(4));
        assert_eq!(tracker.best().height, 2);
    }

    #[test]
    fn test_prune_finalized_linear_chain() {
        // 0 -> 1 -> 2 -> 3
        // Finalize up to block 2
        let finalized = make_block(0, hash_from_u8(0), hash_from_u8(0));
        let mut tracker = UnfinalizedTracker::new_empty(finalized);

        tracker.attach_block(make_block(1, hash_from_u8(1), hash_from_u8(0)));
        tracker.attach_block(make_block(2, hash_from_u8(2), hash_from_u8(1)));
        tracker.attach_block(make_block(3, hash_from_u8(3), hash_from_u8(2)));

        let report = tracker.prune_finalized(hash_from_u8(2)).unwrap();

        // Blocks 1 and 2 should be finalized
        assert_eq!(report.finalize.len(), 2);
        assert!(report.finalize.contains(&hash_from_u8(1)));
        assert!(report.finalize.contains(&hash_from_u8(2)));

        // No blocks should be removed (all on main chain)
        assert!(report.remove.is_empty());

        // Block 3 should still be tracked, block 2 is kept as finalized, block 1 removed
        assert!(tracker.contains_block(&hash_from_u8(3)));
        assert!(tracker.contains_block(&hash_from_u8(2))); // finalized block is kept
        assert!(!tracker.contains_block(&hash_from_u8(1)));

        // New finalized should be block 2
        assert_eq!(tracker.finalized().hash, hash_from_u8(2));
        assert_eq!(tracker.finalized().height, 2);
    }

    #[test]
    fn test_prune_finalized_with_fork() {
        //     0
        //    / \
        //   1   2
        //   |   |
        //   3   4
        //
        // Finalize block 2, should remove blocks 1 and 3
        let finalized = make_block(0, hash_from_u8(0), hash_from_u8(0));
        let mut tracker = UnfinalizedTracker::new_empty(finalized);

        tracker.attach_block(make_block(1, hash_from_u8(1), hash_from_u8(0)));
        tracker.attach_block(make_block(1, hash_from_u8(2), hash_from_u8(0)));
        tracker.attach_block(make_block(2, hash_from_u8(3), hash_from_u8(1)));
        tracker.attach_block(make_block(2, hash_from_u8(4), hash_from_u8(2)));

        let report = tracker.prune_finalized(hash_from_u8(2)).unwrap();

        // Block 2 should be finalized
        assert_eq!(report.finalize.len(), 1);
        assert!(report.finalize.contains(&hash_from_u8(2)));

        // Blocks 1 and 3 should be removed (not on finalized chain)
        assert_eq!(report.remove.len(), 2);
        assert!(report.remove.contains(&hash_from_u8(1)));
        assert!(report.remove.contains(&hash_from_u8(3)));

        // Only block 4 should remain
        assert!(tracker.contains_block(&hash_from_u8(4)));
        assert!(!tracker.contains_block(&hash_from_u8(1)));
        assert!(!tracker.contains_block(&hash_from_u8(3)));

        assert_eq!(tracker.finalized().hash, hash_from_u8(2));
    }

    #[test]
    fn test_prune_finalized_multiple_forks() {
        //       0
        //      /|\
        //     1 2 3
        //     |   |
        //     4   5
        //
        // Finalize block 3, should remove blocks 1, 2, 4
        let finalized = make_block(0, hash_from_u8(0), hash_from_u8(0));
        let mut tracker = UnfinalizedTracker::new_empty(finalized);

        tracker.attach_block(make_block(1, hash_from_u8(1), hash_from_u8(0)));
        tracker.attach_block(make_block(1, hash_from_u8(2), hash_from_u8(0)));
        tracker.attach_block(make_block(1, hash_from_u8(3), hash_from_u8(0)));
        tracker.attach_block(make_block(2, hash_from_u8(4), hash_from_u8(1)));
        tracker.attach_block(make_block(2, hash_from_u8(5), hash_from_u8(3)));

        let report = tracker.prune_finalized(hash_from_u8(3)).unwrap();

        // Block 3 should be finalized
        assert_eq!(report.finalize.len(), 1);
        assert!(report.finalize.contains(&hash_from_u8(3)));

        // Blocks 1, 2, 4 should be removed
        assert_eq!(report.remove.len(), 3);
        assert!(report.remove.contains(&hash_from_u8(1)));
        assert!(report.remove.contains(&hash_from_u8(2)));
        assert!(report.remove.contains(&hash_from_u8(4)));

        // Only block 5 should remain
        assert!(tracker.contains_block(&hash_from_u8(5)));
        assert_eq!(tracker.finalized().hash, hash_from_u8(3));
    }

    #[test]
    fn test_prune_finalized_noop() {
        let finalized = make_block(0, hash_from_u8(0), hash_from_u8(0));
        let mut tracker = UnfinalizedTracker::new_empty(finalized);

        tracker.attach_block(make_block(1, hash_from_u8(1), hash_from_u8(0)));

        // Try to finalize the already finalized block
        let report = tracker.prune_finalized(hash_from_u8(0)).unwrap();

        // Nothing should change
        assert!(report.finalize.is_empty());
        assert!(report.remove.is_empty());
        assert_eq!(tracker.finalized().hash, hash_from_u8(0));
        assert!(tracker.contains_block(&hash_from_u8(1)));
    }

    #[test]
    fn test_prune_finalized_unknown_block() {
        let finalized = make_block(0, hash_from_u8(0), hash_from_u8(0));
        let mut tracker = UnfinalizedTracker::new_empty(finalized);

        tracker.attach_block(make_block(1, hash_from_u8(1), hash_from_u8(0)));

        // Try to finalize an unknown block
        let result = tracker.prune_finalized(hash_from_u8(99));

        assert!(matches!(
            result,
            Err(UnfinalizedTrackerError::UnknownBlock(_))
        ));
    }

    #[test]
    fn test_is_canonical_linear_chain() {
        // 0 -> 1 -> 2 -> 3
        let finalized = make_block(0, hash_from_u8(0), hash_from_u8(0));
        let mut tracker = UnfinalizedTracker::new_empty(finalized);

        tracker.attach_block(make_block(1, hash_from_u8(1), hash_from_u8(0)));
        tracker.attach_block(make_block(2, hash_from_u8(2), hash_from_u8(1)));
        tracker.attach_block(make_block(3, hash_from_u8(3), hash_from_u8(2)));

        // All blocks should be canonical
        assert!(tracker.is_canonical(&hash_from_u8(0))); // finalized
        assert!(tracker.is_canonical(&hash_from_u8(1)));
        assert!(tracker.is_canonical(&hash_from_u8(2)));
        assert!(tracker.is_canonical(&hash_from_u8(3))); // best tip

        // Unknown block should not be canonical
        assert!(!tracker.is_canonical(&hash_from_u8(99)));
    }

    #[test]
    fn test_is_canonical_with_fork() {
        //     0
        //    / \
        //   1   2
        //   |
        //   3 (best)
        let finalized = make_block(0, hash_from_u8(0), hash_from_u8(0));
        let mut tracker = UnfinalizedTracker::new_empty(finalized);

        tracker.attach_block(make_block(1, hash_from_u8(1), hash_from_u8(0)));
        tracker.attach_block(make_block(1, hash_from_u8(2), hash_from_u8(0)));
        tracker.attach_block(make_block(2, hash_from_u8(3), hash_from_u8(1)));

        // Best tip is 3 (height 2)
        assert_eq!(tracker.best().hash, hash_from_u8(3));

        // Blocks on canonical chain (0 -> 1 -> 3)
        assert!(tracker.is_canonical(&hash_from_u8(0))); // finalized
        assert!(tracker.is_canonical(&hash_from_u8(1)));
        assert!(tracker.is_canonical(&hash_from_u8(3))); // best tip

        // Block 2 is on a side chain, not canonical
        assert!(!tracker.is_canonical(&hash_from_u8(2)));
    }

    #[test]
    fn test_is_canonical_multiple_forks() {
        //       0
        //      /|\
        //     1 2 3
        //     |   |
        //     4   5 (best, height 2)
        let finalized = make_block(0, hash_from_u8(0), hash_from_u8(0));
        let mut tracker = UnfinalizedTracker::new_empty(finalized);

        tracker.attach_block(make_block(1, hash_from_u8(1), hash_from_u8(0)));
        tracker.attach_block(make_block(1, hash_from_u8(2), hash_from_u8(0)));
        tracker.attach_block(make_block(1, hash_from_u8(3), hash_from_u8(0)));
        tracker.attach_block(make_block(2, hash_from_u8(4), hash_from_u8(1)));
        tracker.attach_block(make_block(2, hash_from_u8(5), hash_from_u8(3)));

        // Both 4 and 5 are at height 2, but one will be best
        let best = tracker.best().hash;

        // Finalized is always canonical
        assert!(tracker.is_canonical(&hash_from_u8(0)));

        // Check canonical path based on which tip is best
        if best == hash_from_u8(4) {
            assert!(tracker.is_canonical(&hash_from_u8(1)));
            assert!(tracker.is_canonical(&hash_from_u8(4)));
            assert!(!tracker.is_canonical(&hash_from_u8(2)));
            assert!(!tracker.is_canonical(&hash_from_u8(3)));
            assert!(!tracker.is_canonical(&hash_from_u8(5)));
        } else {
            assert!(tracker.is_canonical(&hash_from_u8(3)));
            assert!(tracker.is_canonical(&hash_from_u8(5)));
            assert!(!tracker.is_canonical(&hash_from_u8(1)));
            assert!(!tracker.is_canonical(&hash_from_u8(2)));
            assert!(!tracker.is_canonical(&hash_from_u8(4)));
        }
    }

    #[test]
    fn test_is_canonical_only_finalized() {
        let finalized = make_block(0, hash_from_u8(0), hash_from_u8(0));
        let tracker = UnfinalizedTracker::new_empty(finalized);

        // Only the finalized block exists
        assert!(tracker.is_canonical(&hash_from_u8(0)));
        assert!(!tracker.is_canonical(&hash_from_u8(1)));
    }

    #[test]
    fn test_prune_finalized_deep_chain() {
        // 0 -> 1 -> 2 -> 3 -> 4 -> 5
        // Finalize up to block 4
        let finalized = make_block(0, hash_from_u8(0), hash_from_u8(0));
        let mut tracker = UnfinalizedTracker::new_empty(finalized);

        for i in 1..=5 {
            tracker.attach_block(make_block(
                i,
                hash_from_u8(i as u8),
                hash_from_u8((i - 1) as u8),
            ));
        }

        let report = tracker.prune_finalized(hash_from_u8(4)).unwrap();

        // Blocks 1, 2, 3, 4 should be finalized
        assert_eq!(report.finalize.len(), 4);
        assert!(report.finalize.contains(&hash_from_u8(1)));
        assert!(report.finalize.contains(&hash_from_u8(2)));
        assert!(report.finalize.contains(&hash_from_u8(3)));
        assert!(report.finalize.contains(&hash_from_u8(4)));

        // No blocks removed (linear chain)
        assert!(report.remove.is_empty());

        // Only block 5 should remain
        assert!(tracker.contains_block(&hash_from_u8(5)));
        assert_eq!(tracker.finalized().hash, hash_from_u8(4));
        assert_eq!(tracker.finalized().height, 4);
    }
}
