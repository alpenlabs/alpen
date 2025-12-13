use std::collections::HashMap;

use alpen_ee_common::{
    chain_tracker::{AppendResult, ChainTracker, ChainTrackerError, PruneReport},
    ExecBlockRecord, ExecBlockStorage, StorageError,
};
use strata_acct_types::Hash;
use thiserror::Error;

/// Errors that can occur in the execution chain state.
#[derive(Debug, Error)]
pub enum ExecChainStateError {
    /// Block not found
    #[error("missing expected block: {0:?}")]
    MissingBlock(Hash),
    /// exec finalized chain should not be empty
    #[error("expected exec finalized chain genesis block to be present")]
    MissingGenesisBlock,
    /// Storage error
    #[error(transparent)]
    Storage(#[from] StorageError),
    /// Chain tracker error
    #[error("chain tracker error: {0}")]
    ChainTracker(String),
}

impl<Id: std::fmt::Debug> From<ChainTrackerError<Id>> for ExecChainStateError {
    fn from(err: ChainTrackerError<Id>) -> Self {
        ExecChainStateError::ChainTracker(err.to_string())
    }
}

/// Manages the execution chain state, tracking both unfinalized blocks and orphans.
///
/// Uses the generic `ChainTracker` internally while maintaining a cache of full block records.
#[derive(Debug)]
pub struct ExecChainState {
    /// Generic chain tracker for block structure
    tracker: ChainTracker<ExecBlockRecord>,
    /// Cached block data for quick access
    blocks: HashMap<Hash, ExecBlockRecord>,
}

impl ExecChainState {
    /// Create new chain tracker with last finalized block
    pub(crate) fn new_empty(finalized_block: ExecBlockRecord) -> Self {
        let blockhash = finalized_block.blockhash();
        Self {
            tracker: ChainTracker::new(finalized_block.clone()),
            blocks: HashMap::from([(blockhash, finalized_block)]),
        }
    }

    /// Returns the hash of the current best chain tip.
    pub fn tip_blockhash(&self) -> Hash {
        *self.tracker.tip_id()
    }

    /// Returns the hash of the current finalized block.
    pub fn finalized_blockhash(&self) -> Hash {
        *self.tracker.finalized_id()
    }

    /// Appends a new block to the chain state.
    ///
    /// Attempts to attach the block to the unfinalized chain. If successful, checks if any
    /// orphan blocks can now be attached. Returns the new tip hash.
    pub(crate) fn append_block(&mut self, block: ExecBlockRecord) -> Hash {
        let blockhash = block.blockhash();

        match self.tracker.append(block.clone()) {
            AppendResult::Attached(new_tip) => {
                self.blocks.insert(blockhash, block);
                new_tip
            }
            AppendResult::AlreadyExists => self.tip_blockhash(),
            AppendResult::BelowFinalized => self.tip_blockhash(),
            AppendResult::Orphaned => {
                self.blocks.insert(blockhash, block);
                self.tip_blockhash()
            }
        }
    }

    /// Returns the current best block record.
    pub(crate) fn get_best_block(&self) -> &ExecBlockRecord {
        self.blocks
            .get(self.tracker.tip_id())
            .expect("best block should exist in cache")
    }

    /// Checks if a block exists in the unfinalized tracker.
    pub(crate) fn contains_unfinalized_block(&self, hash: &Hash) -> bool {
        self.tracker.contains_unfinalized(hash)
    }

    /// Checks if a block exists in the orphan tracker.
    pub(crate) fn contains_orphan_block(&self, hash: &Hash) -> bool {
        self.tracker.contains_orphan(hash)
    }

    /// Returns the canonical chain from finalized (exclusive) to tip (inclusive).
    pub fn canonical_chain(&self) -> &[Hash] {
        self.tracker.canonical_chain()
    }

    /// Returns the block hash at the given height on the canonical chain.
    pub fn canonical_blockhash_at_height(&self, height: u64) -> Option<&Hash> {
        self.tracker.canonical_id_at_index(height)
    }

    /// Checks if a block is on the canonical chain.
    pub fn is_canonical(&self, hash: &Hash) -> bool {
        self.tracker.is_canonical(hash)
    }

    /// Advances finalization to the given block and prunes stale blocks.
    ///
    /// Removes finalized blocks and blocks that no longer extend the finalized chain,
    /// as well as old orphans at or below the finalized height.
    pub(crate) fn prune_finalized(
        &mut self,
        finalized: Hash,
    ) -> Result<PruneReport<Hash>, ExecChainStateError> {
        let report = self.tracker.prune_to(finalized)?;

        // Remove finalized blocks from cache (they're now in the DB)
        for hash in &report.finalized {
            self.blocks.remove(hash);
        }

        // Remove pruned blocks from cache
        for hash in &report.pruned {
            self.blocks.remove(hash);
        }

        Ok(report)
    }
}

/// Initializes chain state from storage using the last finalized block and all unfinalized blocks.
pub async fn init_exec_chain_state_from_storage<TStorage: ExecBlockStorage>(
    storage: &TStorage,
) -> Result<ExecChainState, ExecChainStateError> {
    // Note: This function is expected to be run after
    // `alpen_ee_genesis::handle_finalized_exec_genesis` which ensures there is at least genesis
    // block written to the db if it was originally empty.
    // If the db is still empty at this point, something really unexpected has happened, and we
    // cannot continue normal execution.
    let last_finalized_block = storage
        .best_finalized_block()
        .await?
        .ok_or(ExecChainStateError::MissingGenesisBlock)?;

    let mut state = ExecChainState::new_empty(last_finalized_block);

    for blockhash in storage.get_unfinalized_blocks().await? {
        let block = storage
            .get_exec_block(blockhash)
            .await?
            .ok_or(ExecChainStateError::MissingBlock(blockhash))?;

        state.append_block(block);
    }

    Ok(state)
}

#[cfg(test)]
mod tests {
    use strata_acct_types::BitcoinAmount;
    use strata_ee_acct_types::EeAccountState;
    use strata_ee_chain_types::{BlockInputs, BlockOutputs, ExecBlockCommitment, ExecBlockPackage};
    use strata_identifiers::{Buf32, OLBlockCommitment};

    use super::*;

    /// Helper to create a test block with the specified properties
    fn create_test_block(
        blocknum: u64,
        blockhash: Hash,
        parent_blockhash: Hash,
    ) -> ExecBlockRecord {
        let account_state =
            EeAccountState::new(blockhash, BitcoinAmount::ZERO, Vec::new(), Vec::new());

        let package = ExecBlockPackage::new(
            ExecBlockCommitment::new(blockhash, [0; 32]),
            BlockInputs::new_empty(),
            BlockOutputs::new_empty(),
        );

        let ol_block = OLBlockCommitment::new(0, Buf32::new([0u8; 32]).into());

        ExecBlockRecord::new(
            package,
            account_state,
            blocknum,
            ol_block,
            0,
            parent_blockhash,
        )
    }

    /// Helper to create a hash from a u8 value
    fn hash_from_u8(value: u8) -> Hash {
        Hash::from(Buf32::new([value; 32]))
    }

    #[test]
    fn test_append_block_linear_chain() {
        let block_a = create_test_block(0, hash_from_u8(0), hash_from_u8(0));
        let mut state = ExecChainState::new_empty(block_a.clone());

        let block_b = create_test_block(1, hash_from_u8(1), hash_from_u8(0));
        let tip = state.append_block(block_b.clone());

        assert_eq!(tip, hash_from_u8(1));
        assert_eq!(state.tip_blockhash(), hash_from_u8(1));
        assert!(state.contains_unfinalized_block(&hash_from_u8(1)));
    }

    #[test]
    fn test_append_orphan_then_parent() {
        let block_a = create_test_block(0, hash_from_u8(0), hash_from_u8(0));
        let mut state = ExecChainState::new_empty(block_a.clone());

        // Add orphan block C (parent B is missing)
        let block_c = create_test_block(2, hash_from_u8(2), hash_from_u8(1));
        let tip = state.append_block(block_c.clone());

        // Tip should still be A
        assert_eq!(tip, hash_from_u8(0));
        assert!(state.contains_orphan_block(&hash_from_u8(2)));
        assert!(!state.contains_unfinalized_block(&hash_from_u8(2)));

        // Add parent block B - should trigger C to be attached
        let block_b = create_test_block(1, hash_from_u8(1), hash_from_u8(0));
        let tip = state.append_block(block_b.clone());

        // Now tip should be C (block 2)
        assert_eq!(tip, hash_from_u8(2));
        assert!(!state.contains_orphan_block(&hash_from_u8(2)));
        assert!(state.contains_unfinalized_block(&hash_from_u8(2)));
    }

    #[test]
    fn test_orphan_chain_reattachment() {
        let block_a = create_test_block(0, hash_from_u8(0), hash_from_u8(0));
        let mut state = ExecChainState::new_empty(block_a.clone());

        // Add chain of orphans: D -> C -> B (all missing parent)
        let block_d = create_test_block(3, hash_from_u8(3), hash_from_u8(2));
        let block_c = create_test_block(2, hash_from_u8(2), hash_from_u8(1));
        let block_b = create_test_block(1, hash_from_u8(1), hash_from_u8(0));

        state.append_block(block_d.clone());
        state.append_block(block_c.clone());

        // All should be orphans
        assert!(state.contains_orphan_block(&hash_from_u8(3)));
        assert!(state.contains_orphan_block(&hash_from_u8(2)));

        // Add B - should cascade attach C and D
        let tip = state.append_block(block_b.clone());

        assert_eq!(tip, hash_from_u8(3));
        assert!(!state.contains_orphan_block(&hash_from_u8(1)));
        assert!(!state.contains_orphan_block(&hash_from_u8(2)));
        assert!(!state.contains_orphan_block(&hash_from_u8(3)));
        assert!(state.contains_unfinalized_block(&hash_from_u8(1)));
        assert!(state.contains_unfinalized_block(&hash_from_u8(2)));
        assert!(state.contains_unfinalized_block(&hash_from_u8(3)));
    }

    #[test]
    fn test_orphan_on_side_chain() {
        //
        // Chain structure:
        //        A (finalized)
        //       / \
        //      B   D (side chain)
        //      |   |
        //      C   E (orphan, child of D)
        //
        // When we add D to the side chain, the best tip is still C.
        // check_orphan_blocks should look for children of D (the block just attached),
        // not just children of C (the best tip), so E gets properly attached.

        let block_a = create_test_block(0, hash_from_u8(0), hash_from_u8(0));
        let mut state = ExecChainState::new_empty(block_a.clone());

        // Build main chain: A -> B -> C
        let block_b = create_test_block(1, hash_from_u8(1), hash_from_u8(0));
        let block_c = create_test_block(2, hash_from_u8(2), hash_from_u8(1));
        state.append_block(block_b.clone());
        state.append_block(block_c.clone());

        // C should be the tip
        assert_eq!(state.tip_blockhash(), hash_from_u8(2));

        // Add orphan E (child of D, which doesn't exist yet)
        let block_e = create_test_block(2, hash_from_u8(4), hash_from_u8(3));
        state.append_block(block_e.clone());

        // E should be an orphan
        assert!(state.contains_orphan_block(&hash_from_u8(4)));

        // Add D (side chain from A)
        let block_d = create_test_block(1, hash_from_u8(3), hash_from_u8(0));
        let tip = state.append_block(block_d.clone());

        // E should have been attached
        assert!(!state.contains_orphan_block(&hash_from_u8(4)));
        assert!(state.contains_unfinalized_block(&hash_from_u8(4)));

        // The tip should still be C since it's taller
        assert_eq!(tip, hash_from_u8(2));
    }

    #[test]
    fn test_multiple_orphan_branches() {
        //
        //          A
        //        / | \
        //       B  D  F
        //       |  |  |
        //       C  E  G

        let block_a = create_test_block(0, hash_from_u8(0), hash_from_u8(0));
        let mut state = ExecChainState::new_empty(block_a.clone());

        // Add main chain B -> C
        let block_b = create_test_block(1, hash_from_u8(1), hash_from_u8(0));
        let block_c = create_test_block(2, hash_from_u8(2), hash_from_u8(1));
        state.append_block(block_b);
        state.append_block(block_c);

        // Add orphans E and G
        let block_e = create_test_block(2, hash_from_u8(4), hash_from_u8(3));
        let block_g = create_test_block(2, hash_from_u8(6), hash_from_u8(5));
        state.append_block(block_e.clone());
        state.append_block(block_g.clone());

        assert!(state.contains_orphan_block(&hash_from_u8(4)));
        assert!(state.contains_orphan_block(&hash_from_u8(6)));

        // Add D - should attach E
        let block_d = create_test_block(1, hash_from_u8(3), hash_from_u8(0));
        state.append_block(block_d);

        assert!(!state.contains_orphan_block(&hash_from_u8(4)));
        assert!(state.contains_unfinalized_block(&hash_from_u8(4)));

        // Add F - should attach G
        let block_f = create_test_block(1, hash_from_u8(5), hash_from_u8(0));
        state.append_block(block_f);

        assert!(!state.contains_orphan_block(&hash_from_u8(6)));
        assert!(state.contains_unfinalized_block(&hash_from_u8(6)));
    }

    #[test]
    fn test_deep_orphan_chain_on_side_branch() {
        //
        //      A
        //     / \
        //    B   D -> E -> F -> G
        //
        // Add orphans in reverse order: G, F, E
        // Then add D, which should cascade attach E, F, G

        let block_a = create_test_block(0, hash_from_u8(0), hash_from_u8(0));
        let mut state = ExecChainState::new_empty(block_a.clone());

        // Add main chain B
        let block_b = create_test_block(1, hash_from_u8(1), hash_from_u8(0));
        state.append_block(block_b);

        // Add deep orphan chain (in reverse)
        let block_g = create_test_block(4, hash_from_u8(6), hash_from_u8(5));
        let block_f = create_test_block(3, hash_from_u8(5), hash_from_u8(4));
        let block_e = create_test_block(2, hash_from_u8(4), hash_from_u8(3));

        state.append_block(block_g.clone());
        state.append_block(block_f.clone());
        state.append_block(block_e.clone());

        assert!(state.contains_orphan_block(&hash_from_u8(4)));
        assert!(state.contains_orphan_block(&hash_from_u8(5)));
        assert!(state.contains_orphan_block(&hash_from_u8(6)));

        // Add D - should cascade attach E, then F, then G
        let block_d = create_test_block(1, hash_from_u8(3), hash_from_u8(0));
        let tip = state.append_block(block_d);

        // All blocks should now be attached
        assert!(!state.contains_orphan_block(&hash_from_u8(4)));
        assert!(!state.contains_orphan_block(&hash_from_u8(5)));
        assert!(!state.contains_orphan_block(&hash_from_u8(6)));

        assert!(state.contains_unfinalized_block(&hash_from_u8(4)));
        assert!(state.contains_unfinalized_block(&hash_from_u8(5)));
        assert!(state.contains_unfinalized_block(&hash_from_u8(6)));

        // The tip should be G since it's the tallest
        assert_eq!(tip, hash_from_u8(6));
    }

    #[test]
    fn test_prune_finalized_simple() {
        // A -> B -> C -> D
        // Finalize up to C
        let block_a = create_test_block(0, hash_from_u8(0), hash_from_u8(0));
        let mut state = ExecChainState::new_empty(block_a.clone());

        let block_b = create_test_block(1, hash_from_u8(1), hash_from_u8(0));
        let block_c = create_test_block(2, hash_from_u8(2), hash_from_u8(1));
        let block_d = create_test_block(3, hash_from_u8(3), hash_from_u8(2));

        state.append_block(block_b);
        state.append_block(block_c);
        state.append_block(block_d);

        // Finalize block C
        state.prune_finalized(hash_from_u8(2)).unwrap();

        // A, B should be removed, C kept as finalized, D remains unfinalized
        assert_eq!(state.finalized_blockhash(), hash_from_u8(2));
        assert!(state.contains_unfinalized_block(&hash_from_u8(3)));
        assert!(!state.contains_unfinalized_block(&hash_from_u8(1)));
    }

    #[test]
    fn test_prune_finalized_with_fork_removes_side_chain() {
        //     A
        //    / \
        //   B   D
        //   |   |
        //   C   E
        //
        // Finalize B, should remove D and E
        let block_a = create_test_block(0, hash_from_u8(0), hash_from_u8(0));
        let mut state = ExecChainState::new_empty(block_a.clone());

        let block_b = create_test_block(1, hash_from_u8(1), hash_from_u8(0));
        let block_c = create_test_block(2, hash_from_u8(2), hash_from_u8(1));
        let block_d = create_test_block(1, hash_from_u8(3), hash_from_u8(0));
        let block_e = create_test_block(2, hash_from_u8(4), hash_from_u8(3));

        state.append_block(block_b);
        state.append_block(block_c);
        state.append_block(block_d);
        state.append_block(block_e);

        // All blocks should be present
        assert!(state.contains_unfinalized_block(&hash_from_u8(1)));
        assert!(state.contains_unfinalized_block(&hash_from_u8(2)));
        assert!(state.contains_unfinalized_block(&hash_from_u8(3)));
        assert!(state.contains_unfinalized_block(&hash_from_u8(4)));

        // Finalize block B
        state.prune_finalized(hash_from_u8(1)).unwrap();

        // B is now finalized, C remains, D and E should be removed
        assert_eq!(state.finalized_blockhash(), hash_from_u8(1));
        assert!(state.contains_unfinalized_block(&hash_from_u8(2)));
        assert!(!state.contains_unfinalized_block(&hash_from_u8(3)));
        assert!(!state.contains_unfinalized_block(&hash_from_u8(4)));
    }

    #[test]
    fn test_prune_finalized_removes_old_orphans() {
        //   A -> B -> C
        //
        //   Orphans: D (height 1), E (height 2)
        //
        // Finalize C, should remove orphans at or below height 2
        let block_a = create_test_block(0, hash_from_u8(0), hash_from_u8(0));
        let mut state = ExecChainState::new_empty(block_a.clone());

        let block_b = create_test_block(1, hash_from_u8(1), hash_from_u8(0));
        let block_c = create_test_block(2, hash_from_u8(2), hash_from_u8(1));

        // Add orphans
        let orphan_d = create_test_block(1, hash_from_u8(10), hash_from_u8(99));
        let orphan_e = create_test_block(2, hash_from_u8(11), hash_from_u8(99));
        let orphan_f = create_test_block(3, hash_from_u8(12), hash_from_u8(99));

        state.append_block(block_b);
        state.append_block(block_c);
        state.append_block(orphan_d);
        state.append_block(orphan_e);
        state.append_block(orphan_f);

        // All orphans should be present
        assert!(state.contains_orphan_block(&hash_from_u8(10)));
        assert!(state.contains_orphan_block(&hash_from_u8(11)));
        assert!(state.contains_orphan_block(&hash_from_u8(12)));

        // Finalize block C (height 2)
        state.prune_finalized(hash_from_u8(2)).unwrap();

        // Orphans at or below height 2 should be removed
        assert!(!state.contains_orphan_block(&hash_from_u8(10)));
        assert!(!state.contains_orphan_block(&hash_from_u8(11)));
        assert!(state.contains_orphan_block(&hash_from_u8(12))); // height 3, kept
    }

    #[test]
    fn test_prune_finalized_updates_tip() {
        //     A
        //    / \
        //   B   D
        //   |
        //   C (tip)
        //
        // Finalize D, should remove B and C, tip becomes D
        let block_a = create_test_block(0, hash_from_u8(0), hash_from_u8(0));
        let mut state = ExecChainState::new_empty(block_a.clone());

        let block_b = create_test_block(1, hash_from_u8(1), hash_from_u8(0));
        let block_c = create_test_block(2, hash_from_u8(2), hash_from_u8(1));
        let block_d = create_test_block(1, hash_from_u8(3), hash_from_u8(0));

        state.append_block(block_b);
        state.append_block(block_c);
        state.append_block(block_d);

        // Tip should be C (height 2)
        assert_eq!(state.tip_blockhash(), hash_from_u8(2));

        // Finalize D
        state.prune_finalized(hash_from_u8(3)).unwrap();

        // Tip should now be D (the finalized block, since there are no unfinalized blocks)
        assert_eq!(state.finalized_blockhash(), hash_from_u8(3));
        assert_eq!(state.tip_blockhash(), hash_from_u8(3));
    }

    #[test]
    fn test_canonical_chain() {
        //     A (finalized)
        //    / \
        //   B   D
        //   |
        //   C
        let block_a = create_test_block(0, hash_from_u8(0), hash_from_u8(0));
        let mut state = ExecChainState::new_empty(block_a.clone());

        let block_b = create_test_block(1, hash_from_u8(1), hash_from_u8(0));
        let block_c = create_test_block(2, hash_from_u8(2), hash_from_u8(1));
        let block_d = create_test_block(1, hash_from_u8(3), hash_from_u8(0));

        state.append_block(block_b);
        state.append_block(block_c);
        state.append_block(block_d);

        // Canonical chain should be A -> B -> C
        let canonical = state.canonical_chain();
        assert_eq!(canonical, &[hash_from_u8(1), hash_from_u8(2)]);

        assert!(state.is_canonical(&hash_from_u8(0))); // finalized
        assert!(state.is_canonical(&hash_from_u8(1)));
        assert!(state.is_canonical(&hash_from_u8(2)));
        assert!(!state.is_canonical(&hash_from_u8(3))); // side chain

        assert_eq!(
            state.canonical_blockhash_at_height(0),
            Some(&hash_from_u8(0))
        );
        assert_eq!(
            state.canonical_blockhash_at_height(1),
            Some(&hash_from_u8(1))
        );
        assert_eq!(
            state.canonical_blockhash_at_height(2),
            Some(&hash_from_u8(2))
        );
        assert_eq!(state.canonical_blockhash_at_height(3), None);
    }
}
