use std::collections::HashSet;

use strata_primitives::l1::L1Block;
use strata_state::l1::L1BlockId;
use strata_storage::NodeStorage;
use tracing::warn;

use super::common::{IndexedBlockTable, L1Header};

pub(crate) enum AttachBlockResult {
    Attachable,
    Orphan,
    Duplicate,
    BelowSafeHeight,
}

pub(crate) struct ChainTracker {
    // currently tracked tip blocks
    chain_tips: HashSet<L1BlockId>,
    // blocks > safe_height
    chain: IndexedBlockTable,
    // height below which we dont track for reorgs
    safe_height: u64,
    best: Option<L1Header>,
}

impl ChainTracker {
    /// Gets current best block
    pub(crate) fn best(&self) -> Option<&L1Header> {
        self.best.as_ref()
    }

    pub(crate) fn safe_height(&self) -> u64 {
        self.safe_height
    }

    /// Tests whether a given L1 block can be attached to the chain tracker.
    ///
    /// # Arguments
    /// * `block`: A reference to the `L1Block` to test.
    /// # Returns
    /// An `AttachBlockResult` indicating the status of the block relative to the chain tracker.
    pub(crate) fn test_attach_block(&self, block: &L1Block) -> AttachBlockResult {
        if block.height() < self.safe_height {
            return AttachBlockResult::BelowSafeHeight;
        }

        if self.chain.by_block_id.contains_key(&block.block_id()) {
            return AttachBlockResult::Duplicate;
        }

        // if new block extends chain
        if self.chain.by_block_id.contains_key(&block.parent_id()) {
            return AttachBlockResult::Attachable;
        }

        AttachBlockResult::Orphan
    }

    /// Attaches a block to the chain tracker without performing prior validation checks.
    ///
    /// This function assumes that the caller has already determined that the block
    /// is attachable (e.g., its parent exists in the chain). It updates the
    /// `chain_tips` and inserts the block into the internal `chain` structure.
    ///
    /// After attaching the block, it re-evaluates the best block in the chain.
    ///
    /// # Arguments
    /// * `block`: The `L1Header` to attach to the chain.
    /// # Returns
    /// * `true` if the attached block becomes the new best block.
    /// * `false` if the attached block does not change the current best block.
    pub(crate) fn attach_block_unchecked(&mut self, block: L1Header) -> bool {
        self.chain_tips.remove(&block.parent_id());
        self.chain_tips.insert(block.block_id());
        self.chain.insert(block);

        let old_best = self.best;
        self.best = self.find_best_block();
        self.best != old_best
    }

    /// Prunes the chain tracker, removing blocks with a height less than `min_height`.
    ///
    /// # Arguments
    /// * `min_height`: The minimum block height to retain. Blocks below this height will be
    ///   removed.
    /// # Returns
    /// The number of blocks that were pruned from the chain.
    pub(crate) fn prune(&mut self, min_height: u64) -> usize {
        let Some(best) = self.best.as_ref() else {
            // chain tracker is empty
            debug_assert!(self.chain_tips.is_empty());
            debug_assert!(self.chain.by_block_id.is_empty());
            return 0;
        };

        // ensure best block is never pruned
        if min_height > best.height() {
            warn!(best_height = %best.height(), prune_height = %min_height, "csm: attempt to purge above best block");
            return 0;
        }

        let pruned = self.chain.prune_to_height(min_height);
        self.chain_tips
            .retain(|block_id| !pruned.contains(block_id));

        // set new safe_height
        self.safe_height = min_height;

        pruned.len()
    }

    /// Find block with highest accumulated POW among tracked blocks
    fn find_best_block(&self) -> Option<L1Header> {
        self.chain_tips.iter().fold(self.best, |current_best_opt, tip_id| {
            let tip_header = self.chain.by_block_id.get(tip_id).copied()
                .unwrap_or_else(|| panic!("invariant violation: Chain tip ID {:?} not found in chain.by_block_id.", tip_id));

            match current_best_opt {
                Some(best_header_so_far) => {
                    if tip_header.accumulated_pow() > best_header_so_far.accumulated_pow() {
                        Some(tip_header) // New tip is better
                    } else {
                        current_best_opt // Existing best is still better or equal
                    }
                }
                None => Some(tip_header), // This tip is the first one considered (or self.best was None)
            }
        })
    }
}

pub(crate) fn init_chain_tracker(storage: &NodeStorage) -> anyhow::Result<ChainTracker> {
    todo!()
}
