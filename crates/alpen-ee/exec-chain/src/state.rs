use std::collections::{HashMap, VecDeque};

use alpen_ee_common::{ExecBlockRecord, ExecBlockStorage, StorageError};
use strata_acct_types::Hash;
use thiserror::Error;
use tracing::warn;

use crate::{
    orphan_tracker::OrphanTracker,
    unfinalized_tracker::{AttachBlockRes, UnfinalizedTracker, UnfinalizedTrackerError},
};

/// Errors that can occur in the execution chain state.
#[derive(Debug, Error)]
pub enum ExecChainStateError {
    /// Block is below finalized height
    #[error("block height below finalized")]
    BelowFinalized,
    /// Block not found
    #[error("missing expected block: {0:?}")]
    MissingBlock(Hash),
    /// exec finalized chain should not be empty
    #[error("expected exec finalized chain genesis block to be present")]
    MissingGenesisBlock,
    /// Storage error
    #[error(transparent)]
    Storage(#[from] StorageError),
    /// Unfinalized tracker error
    #[error(transparent)]
    UnfinalizedTracker(#[from] UnfinalizedTrackerError),
}

/// Manages the execution chain state, tracking both unfinalized blocks and orphans.
///
/// Coordinates between the unfinalized tracker (for blocks extending the chain)
/// and the orphan tracker (for blocks whose parent is not yet known).
#[derive(Debug)]
pub struct ExecChainState {
    /// Unfinalized block chains extending from the last finalized block
    unfinalized: UnfinalizedTracker,
    /// Orphan blocks waiting for their parent to arrive
    orphans: OrphanTracker,
    /// Cached block data for quick access
    blocks: HashMap<Hash, ExecBlockRecord>,
}

impl ExecChainState {
    /// Create new chain tracker with last finalized block
    pub(crate) fn new_empty(finalized_block: ExecBlockRecord) -> Self {
        Self {
            unfinalized: UnfinalizedTracker::new_empty((&finalized_block).into()),
            orphans: OrphanTracker::new_empty(),
            blocks: HashMap::from([(finalized_block.blockhash(), finalized_block)]),
        }
    }

    /// Returns the hash of the current best chain tip.
    pub fn tip_blockhash(&self) -> Hash {
        self.unfinalized.best().hash
    }

    /// Returns the hash of the current finalized block.
    pub fn finalized_blockhash(&self) -> Hash {
        self.unfinalized.finalized().hash
    }

    /// Appends a new block to the chain state.
    ///
    /// Attempts to attach the block to the unfinalized chain. If successful, checks if any
    /// orphan blocks can now be attached. Returns the new tip hash.
    pub(crate) fn append_block(
        &mut self,
        block: ExecBlockRecord,
    ) -> Result<Hash, ExecChainStateError> {
        let blockhash = block.blockhash();
        match self.unfinalized.attach_block((&block).into()) {
            AttachBlockRes::Ok(_new_tip) => {
                self.blocks.insert(blockhash, block);
                Ok(self.check_orphan_blocks(blockhash))
            }
            AttachBlockRes::BelowFinalized(_) => Err(ExecChainStateError::BelowFinalized),
            AttachBlockRes::ExistingBlock => {
                warn!("block already present in tracker");
                Ok(self.tip_blockhash())
            }
            AttachBlockRes::OrphanBlock(block_entry) => {
                self.blocks.insert(blockhash, block);
                self.orphans.insert(block_entry);
                Ok(self.tip_blockhash())
            }
        }
    }

    /// Attempts to attach orphan blocks after a new block is added.
    ///
    /// Recursively checks if any orphans can now be attached to the chain,
    /// updating the tip as orphans are connected.
    fn check_orphan_blocks(&mut self, mut tip: Hash) -> Hash {
        let mut attachable_blocks: VecDeque<_> = self.orphans.take_children(&tip).into();
        while let Some(block) = attachable_blocks.pop_front() {
            let blockhash = block.blockhash;
            match self.unfinalized.attach_block(block) {
                AttachBlockRes::Ok(best) => {
                    tip = best;
                    attachable_blocks.append(&mut self.orphans.take_children(&blockhash).into());
                }
                AttachBlockRes::ExistingBlock => {
                    // shouldn't happen but safe to ignore
                    warn!("unexpected existing block");
                }
                AttachBlockRes::OrphanBlock(_) => unreachable!(),
                AttachBlockRes::BelowFinalized(_) => unreachable!(),
            }
        }

        tip
    }

    /// Returns the current best block record.
    pub(crate) fn get_best_block(&self) -> &ExecBlockRecord {
        self.blocks
            .get(&self.unfinalized.best().hash)
            .expect("should exist")
    }

    /// Checks if a block exists in the unfinalized tracker.
    pub(crate) fn contains_unfinalized_block(&self, hash: &Hash) -> bool {
        self.unfinalized.contains_block(hash)
    }

    /// Checks if a block exists in the orphan tracker.
    pub(crate) fn contains_orphan_block(&self, hash: &Hash) -> bool {
        self.orphans.has_block(hash)
    }

    /// Advances finalization to the given block and prunes stale blocks.
    ///
    /// Removes finalized blocks and blocks that no longer extend the finalized chain,
    /// as well as old orphans at or below the finalized height.
    pub(crate) fn prune_finalized(&mut self, finalized: Hash) -> Result<(), ExecChainStateError> {
        let report = self.unfinalized.prune_finalized(finalized)?;
        let finalized_height = self
            .blocks
            .get(&finalized)
            .expect("should exist")
            .blocknum();
        for hash in report.finalize {
            self.blocks.remove(&hash);
        }
        for hash in report.remove {
            self.blocks.remove(&hash);
        }
        let removed_orphans = self.orphans.purge_by_height(finalized_height);
        for hash in removed_orphans {
            self.blocks.remove(&hash);
        }
        Ok(())
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

        state.append_block(block)?;
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
            ExecBlockCommitment::new(blockhash, Hash::new([0; 32])),
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
        let tip = state.append_block(block_b.clone()).unwrap();

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
        let tip = state.append_block(block_c.clone()).unwrap();

        // Tip should still be A
        assert_eq!(tip, hash_from_u8(0));
        assert!(state.contains_orphan_block(&hash_from_u8(2)));
        assert!(!state.contains_unfinalized_block(&hash_from_u8(2)));

        // Add parent block B - should trigger C to be attached
        let block_b = create_test_block(1, hash_from_u8(1), hash_from_u8(0));
        let tip = state.append_block(block_b.clone()).unwrap();

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

        state.append_block(block_d.clone()).unwrap();
        state.append_block(block_c.clone()).unwrap();

        // All should be orphans
        assert!(state.contains_orphan_block(&hash_from_u8(3)));
        assert!(state.contains_orphan_block(&hash_from_u8(2)));

        // Add B - should cascade attach C and D
        let tip = state.append_block(block_b.clone()).unwrap();

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
        state.append_block(block_b.clone()).unwrap();
        state.append_block(block_c.clone()).unwrap();

        // C should be the tip
        assert_eq!(state.tip_blockhash(), hash_from_u8(2));

        // Add orphan E (child of D, which doesn't exist yet)
        let block_e = create_test_block(2, hash_from_u8(4), hash_from_u8(3));
        state.append_block(block_e.clone()).unwrap();

        // E should be an orphan
        assert!(state.contains_orphan_block(&hash_from_u8(4)));

        // Add D (side chain from A)
        let block_d = create_test_block(1, hash_from_u8(3), hash_from_u8(0));
        let tip = state.append_block(block_d.clone()).unwrap();

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
        state.append_block(block_b).unwrap();
        state.append_block(block_c).unwrap();

        // Add orphans E and G
        let block_e = create_test_block(2, hash_from_u8(4), hash_from_u8(3));
        let block_g = create_test_block(2, hash_from_u8(6), hash_from_u8(5));
        state.append_block(block_e.clone()).unwrap();
        state.append_block(block_g.clone()).unwrap();

        assert!(state.contains_orphan_block(&hash_from_u8(4)));
        assert!(state.contains_orphan_block(&hash_from_u8(6)));

        // Add D - should attach E
        let block_d = create_test_block(1, hash_from_u8(3), hash_from_u8(0));
        state.append_block(block_d).unwrap();

        assert!(!state.contains_orphan_block(&hash_from_u8(4)));
        assert!(state.contains_unfinalized_block(&hash_from_u8(4)));

        // Add F - should attach G
        let block_f = create_test_block(1, hash_from_u8(5), hash_from_u8(0));
        state.append_block(block_f).unwrap();

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
        state.append_block(block_b).unwrap();

        // Add deep orphan chain (in reverse)
        let block_g = create_test_block(4, hash_from_u8(6), hash_from_u8(5));
        let block_f = create_test_block(3, hash_from_u8(5), hash_from_u8(4));
        let block_e = create_test_block(2, hash_from_u8(4), hash_from_u8(3));

        state.append_block(block_g.clone()).unwrap();
        state.append_block(block_f.clone()).unwrap();
        state.append_block(block_e.clone()).unwrap();

        assert!(state.contains_orphan_block(&hash_from_u8(4)));
        assert!(state.contains_orphan_block(&hash_from_u8(5)));
        assert!(state.contains_orphan_block(&hash_from_u8(6)));

        // Add D - should cascade attach E, then F, then G
        let block_d = create_test_block(1, hash_from_u8(3), hash_from_u8(0));
        let tip = state.append_block(block_d).unwrap();

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

        state.append_block(block_b).unwrap();
        state.append_block(block_c).unwrap();
        state.append_block(block_d).unwrap();

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

        state.append_block(block_b).unwrap();
        state.append_block(block_c).unwrap();
        state.append_block(block_d).unwrap();
        state.append_block(block_e).unwrap();

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

        state.append_block(block_b).unwrap();
        state.append_block(block_c).unwrap();
        state.append_block(orphan_d).unwrap();
        state.append_block(orphan_e).unwrap();
        state.append_block(orphan_f).unwrap();

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

        state.append_block(block_b).unwrap();
        state.append_block(block_c).unwrap();
        state.append_block(block_d).unwrap();

        // Tip should be C (height 2)
        assert_eq!(state.tip_blockhash(), hash_from_u8(2));

        // Finalize D
        state.prune_finalized(hash_from_u8(3)).unwrap();

        // Tip should now be D (the finalized block, since there are no unfinalized blocks)
        assert_eq!(state.finalized_blockhash(), hash_from_u8(3));
        assert_eq!(state.tip_blockhash(), hash_from_u8(3));
    }
}
