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
