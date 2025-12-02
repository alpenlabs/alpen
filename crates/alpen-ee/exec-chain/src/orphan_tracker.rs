use std::collections::{BTreeMap, HashMap, HashSet};

use strata_acct_types::Hash;

use crate::unfinalized_tracker::BlockEntry;

#[derive(Debug)]
pub(crate) struct OrphanTracker {
    by_hash: HashMap<Hash, BlockEntry>,
    by_parent: HashMap<Hash, HashSet<Hash>>,
    by_height: BTreeMap<u64, HashSet<Hash>>,
}

impl OrphanTracker {
    pub(crate) fn new_empty() -> Self {
        Self {
            by_hash: HashMap::new(),
            by_parent: HashMap::new(),
            by_height: BTreeMap::new(),
        }
    }

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

    pub(crate) fn has_block(&self, hash: &Hash) -> bool {
        self.by_hash.contains_key(hash)
    }

    // pub(crate) fn has_children(&self, parent: &Hash) -> bool {
    //     self.by_parent.contains_key(parent)
    // }

    /// Combined get-and-remove operation.
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

    /// Purge all items with height <= max_height and return the removed hashes.
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
