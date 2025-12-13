//! Orphan tracker for items whose parent is not yet known.

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fmt::Debug,
    hash::Hash,
};

use super::item::ItemEntry;

/// Tracks items whose parent is not yet known (orphans).
///
/// Maintains three indexes for efficient lookup and removal:
/// - by id: direct item access
/// - by parent: finding children when parent arrives
/// - by index: pruning old orphans
#[derive(Debug)]
pub(crate) struct OrphanTracker<Id> {
    /// Item entries indexed by their id
    by_id: HashMap<Id, ItemEntry<Id>>,
    /// Maps parent id to set of child item ids
    by_parent: HashMap<Id, HashSet<Id>>,
    /// Maps item index to set of item ids at that index
    by_index: BTreeMap<u64, HashSet<Id>>,
}

impl<Id: Clone + Eq + Hash + Debug> OrphanTracker<Id> {
    /// Creates a new empty orphan tracker.
    pub(crate) fn new() -> Self {
        Self {
            by_id: HashMap::new(),
            by_parent: HashMap::new(),
            by_index: BTreeMap::new(),
        }
    }

    /// Inserts an item into the tracker, indexing it by id, parent, and index.
    pub(crate) fn insert(&mut self, entry: ItemEntry<Id>) {
        self.by_index
            .entry(entry.index)
            .or_default()
            .insert(entry.id.clone());
        self.by_parent
            .entry(entry.parent_id.clone())
            .or_default()
            .insert(entry.id.clone());
        self.by_id.insert(entry.id.clone(), entry);
    }

    /// Checks if an item with the given id is tracked.
    pub(crate) fn contains(&self, id: &Id) -> bool {
        self.by_id.contains_key(id)
    }

    /// Removes and returns all items that have the specified parent id.
    ///
    /// This is useful when a parent item arrives and we can now process its orphaned children.
    pub(crate) fn take_children(&mut self, parent_id: &Id) -> Vec<ItemEntry<Id>> {
        let Some(child_ids) = self.by_parent.remove(parent_id) else {
            return Vec::new();
        };

        let mut entries = Vec::with_capacity(child_ids.len());
        for id in child_ids {
            let entry = self.by_id.remove(&id).expect("orphan entry should exist");
            let index = entry.index;

            if let Some(by_index) = self.by_index.get_mut(&index) {
                by_index.remove(&id);
                if by_index.is_empty() {
                    self.by_index.remove(&index);
                }
            }

            entries.push(entry);
        }

        entries
    }

    /// Removes all items at or below the specified index and returns their ids.
    ///
    /// This is used to prune old orphans that are unlikely to ever be connected to the chain.
    pub(crate) fn purge_up_to_index(&mut self, max_index: u64) -> Vec<Id> {
        let indices_to_remove: Vec<u64> = self
            .by_index
            .keys()
            .take_while(|&&idx| idx <= max_index)
            .copied()
            .collect();

        let mut removed = Vec::new();

        for index in indices_to_remove {
            let ids = self.by_index.remove(&index).expect("index should exist");
            for id in ids {
                let entry = self.by_id.remove(&id).expect("orphan entry should exist");
                let parent_id = entry.parent_id;

                if let Some(by_parent) = self.by_parent.get_mut(&parent_id) {
                    by_parent.remove(&id);
                    if by_parent.is_empty() {
                        self.by_parent.remove(&parent_id);
                    }
                }

                removed.push(id);
            }
        }

        removed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_entry(index: u64, id: u8, parent_id: u8) -> ItemEntry<u8> {
        ItemEntry {
            index,
            id,
            parent_id,
        }
    }

    #[test]
    fn test_insert_and_contains() {
        let mut tracker = OrphanTracker::new();
        let entry = make_entry(1, 1, 0);

        tracker.insert(entry);

        assert!(tracker.contains(&1));
        assert!(!tracker.contains(&2));
    }

    #[test]
    fn test_take_children_empty() {
        let mut tracker: OrphanTracker<u8> = OrphanTracker::new();
        let children = tracker.take_children(&0);

        assert!(children.is_empty());
    }

    #[test]
    fn test_take_children_single() {
        let mut tracker = OrphanTracker::new();
        let entry = make_entry(1, 1, 0);

        tracker.insert(entry);

        let children = tracker.take_children(&0);

        assert_eq!(children.len(), 1);
        assert_eq!(children[0].id, 1);
        assert!(!tracker.contains(&1));
    }

    #[test]
    fn test_take_children_multiple() {
        //     0
        //   / | \
        //  1  2  3
        let mut tracker = OrphanTracker::new();

        tracker.insert(make_entry(1, 1, 0));
        tracker.insert(make_entry(1, 2, 0));
        tracker.insert(make_entry(1, 3, 0));

        let children = tracker.take_children(&0);

        assert_eq!(children.len(), 3);
        assert!(!tracker.contains(&1));
        assert!(!tracker.contains(&2));
        assert!(!tracker.contains(&3));
    }

    #[test]
    fn test_take_children_removes_only_direct_children() {
        //   0
        //   |
        //   1
        //   |
        //   2
        let mut tracker = OrphanTracker::new();

        tracker.insert(make_entry(1, 1, 0));
        tracker.insert(make_entry(2, 2, 1));

        let children = tracker.take_children(&0);

        assert_eq!(children.len(), 1);
        assert_eq!(children[0].id, 1);

        // Item 2 should still be in the tracker (it's a child of 1, not 0)
        assert!(tracker.contains(&2));
    }

    #[test]
    fn test_purge_up_to_index() {
        let mut tracker = OrphanTracker::new();

        tracker.insert(make_entry(1, 1, 0));
        tracker.insert(make_entry(2, 2, 1));
        tracker.insert(make_entry(3, 3, 2));
        tracker.insert(make_entry(4, 4, 3));

        let removed = tracker.purge_up_to_index(2);

        assert_eq!(removed.len(), 2);
        assert!(removed.contains(&1));
        assert!(removed.contains(&2));

        assert!(!tracker.contains(&1));
        assert!(!tracker.contains(&2));
        assert!(tracker.contains(&3));
        assert!(tracker.contains(&4));
    }

    #[test]
    fn test_purge_up_to_index_empty() {
        let mut tracker = OrphanTracker::new();

        tracker.insert(make_entry(5, 5, 4));
        tracker.insert(make_entry(6, 6, 5));

        let removed = tracker.purge_up_to_index(3);

        assert!(removed.is_empty());
        assert!(tracker.contains(&5));
        assert!(tracker.contains(&6));
    }

    #[test]
    fn test_multiple_orphan_chains() {
        //   0       5
        //   |       |
        //   1       6
        //   |
        //   2
        let mut tracker = OrphanTracker::new();

        tracker.insert(make_entry(1, 1, 0));
        tracker.insert(make_entry(2, 2, 1));
        tracker.insert(make_entry(6, 6, 5));

        // Take children of 0
        let children_0 = tracker.take_children(&0);
        assert_eq!(children_0.len(), 1);
        assert_eq!(children_0[0].id, 1);

        // Items 2 and 6 should still be there
        assert!(tracker.contains(&2));
        assert!(tracker.contains(&6));

        // Take children of 5
        let children_5 = tracker.take_children(&5);
        assert_eq!(children_5.len(), 1);
        assert_eq!(children_5[0].id, 6);

        // Only item 2 should remain
        assert!(tracker.contains(&2));
        assert!(!tracker.contains(&6));
    }
}
