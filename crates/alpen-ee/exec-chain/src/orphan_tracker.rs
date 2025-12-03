use std::collections::{BTreeMap, HashMap, HashSet};

use strata_acct_types::Hash;

use crate::unfinalized_tracker::BlockEntry;

/// Tracks blocks whose parent is not yet known (orphans).
///
/// Maintains three indexes for efficient lookup and removal:
/// - by hash: direct block access
/// - by parent: finding children of a parent block
/// - by height: pruning old orphans
#[derive(Debug)]
pub(crate) struct OrphanTracker {
    /// Block entries indexed by their hash
    by_hash: HashMap<Hash, BlockEntry>,
    /// Maps parent hash to set of child block hashes
    by_parent: HashMap<Hash, HashSet<Hash>>,
    /// Maps block height to set of block hashes at that height
    by_height: BTreeMap<u64, HashSet<Hash>>,
}

impl OrphanTracker {
    /// Creates a new empty orphan tracker.
    pub(crate) fn new_empty() -> Self {
        Self {
            by_hash: HashMap::new(),
            by_parent: HashMap::new(),
            by_height: BTreeMap::new(),
        }
    }

    /// Inserts a block into the tracker, indexing it by hash, parent, and height.
    pub(crate) fn insert(&mut self, block: BlockEntry) {
        self.by_height
            .entry(block.blocknum)
            .or_default()
            .insert(block.blockhash);
        self.by_parent
            .entry(block.parent)
            .or_default()
            .insert(block.blockhash);
        self.by_hash.insert(block.blockhash, block);
    }

    /// Checks if a block with the given hash is tracked.
    pub(crate) fn has_block(&self, hash: &Hash) -> bool {
        self.by_hash.contains_key(hash)
    }

    /// Removes and returns all blocks that have the specified parent hash.
    ///
    /// This is useful when a parent block arrives and we can now process its orphaned children.
    pub(crate) fn take_children(&mut self, parent: &Hash) -> Vec<BlockEntry> {
        let Some(blockhashes) = self.by_parent.remove(parent) else {
            return Vec::new();
        };
        let mut entries = Vec::with_capacity(blockhashes.len());
        for hash in blockhashes {
            let entry = self.by_hash.remove(&hash).expect("should exist");
            let height = entry.blocknum;
            if let Some(by_height) = self.by_height.get_mut(&height) {
                by_height.remove(&hash);
                if by_height.is_empty() {
                    self.by_height.remove_entry(&height);
                }
            }
            entries.push(entry);
        }
        entries
    }

    /// Removes all blocks at or below the specified height and returns their hashes.
    ///
    /// This is used to prune old orphans that are unlikely to ever be connected to the chain.
    pub(crate) fn purge_by_height(&mut self, max_height: u64) -> Vec<Hash> {
        let heights_to_remove: Vec<u64> = self
            .by_height
            .keys()
            .filter(|&&h| h <= max_height)
            .copied()
            .collect();

        let mut removed = Vec::new();

        for height in heights_to_remove {
            let blockhashes = self.by_height.remove(&height).expect("should exist");
            for blockhash in blockhashes {
                let entry = self.by_hash.remove(&blockhash).expect("should exist");
                let parent = entry.parent;
                if let Some(by_parent) = self.by_parent.get_mut(&parent) {
                    by_parent.remove(&blockhash);
                    if by_parent.is_empty() {
                        self.by_parent.remove(&parent);
                    }
                }
                removed.push(blockhash);
            }
        }

        removed
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
    fn test_insert_and_has_block() {
        let mut tracker = OrphanTracker::new_empty();
        let block = make_block(1, hash_from_u8(1), hash_from_u8(0));

        tracker.insert(block);

        assert!(tracker.has_block(&hash_from_u8(1)));
        assert!(!tracker.has_block(&hash_from_u8(2)));
    }

    #[test]
    fn test_take_children_empty() {
        let mut tracker = OrphanTracker::new_empty();
        let children = tracker.take_children(&hash_from_u8(0));

        assert!(children.is_empty());
    }

    #[test]
    fn test_take_children_single() {
        let mut tracker = OrphanTracker::new_empty();
        let block = make_block(1, hash_from_u8(1), hash_from_u8(0));

        tracker.insert(block);

        let children = tracker.take_children(&hash_from_u8(0));

        assert_eq!(children.len(), 1);
        assert_eq!(children[0].blockhash, hash_from_u8(1));
        assert!(!tracker.has_block(&hash_from_u8(1)));
    }

    #[test]
    fn test_take_children_multiple() {
        //     0
        //   / | \
        //  1  2  3
        let mut tracker = OrphanTracker::new_empty();

        tracker.insert(make_block(1, hash_from_u8(1), hash_from_u8(0)));
        tracker.insert(make_block(1, hash_from_u8(2), hash_from_u8(0)));
        tracker.insert(make_block(1, hash_from_u8(3), hash_from_u8(0)));

        let children = tracker.take_children(&hash_from_u8(0));

        assert_eq!(children.len(), 3);
        assert!(!tracker.has_block(&hash_from_u8(1)));
        assert!(!tracker.has_block(&hash_from_u8(2)));
        assert!(!tracker.has_block(&hash_from_u8(3)));
    }

    #[test]
    fn test_take_children_removes_only_direct_children() {
        //   0
        //   |
        //   1
        //   |
        //   2
        let mut tracker = OrphanTracker::new_empty();

        tracker.insert(make_block(1, hash_from_u8(1), hash_from_u8(0)));
        tracker.insert(make_block(2, hash_from_u8(2), hash_from_u8(1)));

        let children = tracker.take_children(&hash_from_u8(0));

        assert_eq!(children.len(), 1);
        assert_eq!(children[0].blockhash, hash_from_u8(1));

        // Block 2 should still be in the tracker (it's a child of 1, not 0)
        assert!(tracker.has_block(&hash_from_u8(2)));
    }

    #[test]
    fn test_purge_by_height() {
        let mut tracker = OrphanTracker::new_empty();

        tracker.insert(make_block(1, hash_from_u8(1), hash_from_u8(0)));
        tracker.insert(make_block(2, hash_from_u8(2), hash_from_u8(1)));
        tracker.insert(make_block(3, hash_from_u8(3), hash_from_u8(2)));
        tracker.insert(make_block(4, hash_from_u8(4), hash_from_u8(3)));

        let removed = tracker.purge_by_height(2);

        assert_eq!(removed.len(), 2);
        assert!(removed.contains(&hash_from_u8(1)));
        assert!(removed.contains(&hash_from_u8(2)));

        assert!(!tracker.has_block(&hash_from_u8(1)));
        assert!(!tracker.has_block(&hash_from_u8(2)));
        assert!(tracker.has_block(&hash_from_u8(3)));
        assert!(tracker.has_block(&hash_from_u8(4)));
    }

    #[test]
    fn test_purge_by_height_empty() {
        let mut tracker = OrphanTracker::new_empty();

        tracker.insert(make_block(5, hash_from_u8(5), hash_from_u8(4)));
        tracker.insert(make_block(6, hash_from_u8(6), hash_from_u8(5)));

        let removed = tracker.purge_by_height(3);

        assert!(removed.is_empty());
        assert!(tracker.has_block(&hash_from_u8(5)));
        assert!(tracker.has_block(&hash_from_u8(6)));
    }

    #[test]
    fn test_multiple_orphan_chains() {
        //   0       5
        //   |       |
        //   1       6
        //   |
        //   2
        let mut tracker = OrphanTracker::new_empty();

        tracker.insert(make_block(1, hash_from_u8(1), hash_from_u8(0)));
        tracker.insert(make_block(2, hash_from_u8(2), hash_from_u8(1)));
        tracker.insert(make_block(6, hash_from_u8(6), hash_from_u8(5)));

        // Take children of 0
        let children_0 = tracker.take_children(&hash_from_u8(0));
        assert_eq!(children_0.len(), 1);
        assert_eq!(children_0[0].blockhash, hash_from_u8(1));

        // Block 2 and 6 should still be there
        assert!(tracker.has_block(&hash_from_u8(2)));
        assert!(tracker.has_block(&hash_from_u8(6)));

        // Take children of 5
        let children_5 = tracker.take_children(&hash_from_u8(5));
        assert_eq!(children_5.len(), 1);
        assert_eq!(children_5[0].blockhash, hash_from_u8(6));

        // Only block 2 should remain
        assert!(tracker.has_block(&hash_from_u8(2)));
        assert!(!tracker.has_block(&hash_from_u8(6)));
    }
}
