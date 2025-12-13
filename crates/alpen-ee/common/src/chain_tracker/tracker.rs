//! Main chain tracker combining unfinalized and orphan trackers.

use std::collections::VecDeque;

use tracing::warn;

use super::{
    error::ChainTrackerError,
    item::{ChainItem, ItemEntry},
    orphan::OrphanTracker,
    unfinalized::{AttachResult, PruneReport, UnfinalizedTracker},
};

/// Result of appending an item to the tracker.
#[derive(Debug)]
pub enum AppendResult<Id> {
    /// Successfully attached to the chain, returns new best tip id
    Attached(Id),
    /// Item already exists in the tracker
    AlreadyExists,
    /// Item index is below finalized, rejected
    BelowFinalized,
    /// Item is orphaned (parent unknown), tracked for later attachment
    Orphaned,
}

/// Tracks a chain of items, managing both unfinalized items and orphans.
///
/// Coordinates between the unfinalized tracker (for items extending the chain)
/// and the orphan tracker (for items whose parent is not yet known).
#[derive(Debug)]
pub struct ChainTracker<Item: ChainItem> {
    /// Tracks the unfinalized chain from finalized tip to best tip
    unfinalized: UnfinalizedTracker<Item::Id>,
    /// Tracks orphan items waiting for their parent
    orphans: OrphanTracker<Item::Id>,
    /// Marker for the Item type
    _item: std::marker::PhantomData<Item>,
}

impl<Item: ChainItem> ChainTracker<Item> {
    /// Creates a new tracker with the finalized item as starting point.
    pub fn new(finalized_item: Item) -> Self {
        Self {
            unfinalized: UnfinalizedTracker::new(finalized_item.as_entry()),
            orphans: OrphanTracker::new(),
            _item: std::marker::PhantomData,
        }
    }

    /// Helper to create a new tracker with finalized item and existing unfinalized items.
    pub fn new_with_unfinalized(finalized_item: Item, unfinalized_items: Vec<Item>) -> Self {
        let mut tracker = Self::new(finalized_item);

        for item in unfinalized_items {
            let _ = tracker.append(item);
        }

        tracker
    }

    /// Returns the current best tip id.
    pub fn tip_id(&self) -> &Item::Id {
        self.unfinalized.best_id()
    }

    /// Returns the current finalized item's id.
    pub fn finalized_id(&self) -> &Item::Id {
        self.unfinalized.finalized_id()
    }

    /// Returns the current finalized item's index.
    pub fn finalized_index(&self) -> u64 {
        self.unfinalized.finalized_index()
    }

    /// Appends a new item to the chain state.
    ///
    /// Attempts to attach the item to the unfinalized chain. If successful, checks if any
    /// orphan items can now be attached.
    pub fn append(&mut self, item: Item) -> AppendResult<Item::Id> {
        let entry = item.as_entry();
        let id = entry.id.clone();

        match self.unfinalized.attach(entry) {
            AttachResult::Attached(_) => {
                let tip = self.process_orphans_after_attach(&id);
                AppendResult::Attached(tip)
            }
            AttachResult::Existing => {
                warn!(?id, "item already present in tracker");
                AppendResult::AlreadyExists
            }
            AttachResult::BelowFinalized => AppendResult::BelowFinalized,
            AttachResult::Orphan => {
                self.orphans.insert(item.as_entry());
                AppendResult::Orphaned
            }
        }
    }

    /// Processes orphans that may now be attachable after a new item was attached.
    ///
    /// Returns the final best tip id after all possible orphans are attached.
    fn process_orphans_after_attach(&mut self, attached_id: &Item::Id) -> Item::Id {
        let mut queue: VecDeque<ItemEntry<Item::Id>> =
            self.orphans.take_children(attached_id).into();

        while let Some(entry) = queue.pop_front() {
            let entry_id = entry.id.clone();

            match self.unfinalized.attach(entry) {
                AttachResult::Attached(_) => {
                    // Check if this newly attached item has orphan children
                    queue.extend(self.orphans.take_children(&entry_id));
                }
                AttachResult::Existing => {
                    warn!(
                        ?entry_id,
                        "unexpected existing item during orphan processing"
                    );
                }
                AttachResult::Orphan | AttachResult::BelowFinalized => {
                    unreachable!("orphan's parent was just attached");
                }
            }
        }

        self.unfinalized.best_id().clone()
    }

    /// Checks if an item is tracked (either unfinalized or orphan).
    pub fn contains(&self, id: &Item::Id) -> bool {
        self.unfinalized.contains(id) || self.orphans.contains(id)
    }

    /// Checks if an item is in the unfinalized tracker.
    pub fn contains_unfinalized(&self, id: &Item::Id) -> bool {
        self.unfinalized.contains(id)
    }

    /// Checks if an item is in the orphan tracker.
    pub fn contains_orphan(&self, id: &Item::Id) -> bool {
        self.orphans.contains(id)
    }

    /// Returns the canonical chain from finalized (exclusive) to tip (inclusive).
    ///
    /// Items are ordered oldest to newest.
    pub fn canonical_chain(&self) -> &[Item::Id] {
        self.unfinalized.canonical_chain()
    }

    /// Returns the id at the given index on the canonical chain.
    ///
    /// Returns None if index is below finalized or above tip.
    pub fn canonical_id_at_index(&self, index: u64) -> Option<&Item::Id> {
        self.unfinalized.canonical_id_at_index(index)
    }

    /// Checks if an item is on the canonical chain.
    pub fn is_canonical(&self, id: &Item::Id) -> bool {
        self.unfinalized.is_canonical(id)
    }

    /// Prunes in-memory state up to the new finalized item.
    ///
    /// The new finalized item must be in the unfinalized tracker.
    /// Also prunes orphans at or below the new finalized index.
    ///
    /// Returns a report of items that were finalized and pruned.
    pub fn prune_to(
        &mut self,
        new_finalized_id: Item::Id,
    ) -> Result<PruneReport<Item::Id>, ChainTrackerError<Item::Id>> {
        let mut report = self.unfinalized.prune_to(new_finalized_id)?;

        // Prune old orphans
        let finalized_index = self.unfinalized.finalized_index();
        let pruned_orphans = self.orphans.purge_up_to_index(finalized_index);
        report.pruned.extend(pruned_orphans);

        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Simple test item implementing ChainItem
    #[derive(Debug, Clone)]
    struct TestItem {
        index: u64,
        id: u8,
        parent_id: u8,
    }

    impl TestItem {
        fn new(index: u64, id: u8, parent_id: u8) -> Self {
            Self {
                index,
                id,
                parent_id,
            }
        }
    }

    impl ChainItem for TestItem {
        type Id = u8;

        fn index(&self) -> u64 {
            self.index
        }

        fn id(&self) -> u8 {
            self.id
        }

        fn parent_id(&self) -> u8 {
            self.parent_id
        }
    }

    #[test]
    fn test_new_and_tip() {
        let finalized = TestItem::new(0, 0, 0);
        let tracker: ChainTracker<TestItem> = ChainTracker::new(finalized);

        assert_eq!(*tracker.tip_id(), 0);
        assert_eq!(*tracker.finalized_id(), 0);
        assert_eq!(tracker.finalized_index(), 0);
    }

    #[test]
    fn test_append_linear_chain() {
        let finalized = TestItem::new(0, 0, 0);
        let mut tracker: ChainTracker<TestItem> = ChainTracker::new(finalized);

        let result = tracker.append(TestItem::new(1, 1, 0));
        assert!(matches!(result, AppendResult::Attached(1)));
        assert_eq!(*tracker.tip_id(), 1);

        let result = tracker.append(TestItem::new(2, 2, 1));
        assert!(matches!(result, AppendResult::Attached(2)));
        assert_eq!(*tracker.tip_id(), 2);
    }

    #[test]
    fn test_append_orphan_then_parent() {
        let finalized = TestItem::new(0, 0, 0);
        let mut tracker: ChainTracker<TestItem> = ChainTracker::new(finalized);

        // Add orphan (parent 1 missing)
        let result = tracker.append(TestItem::new(2, 2, 1));
        assert!(matches!(result, AppendResult::Orphaned));
        assert_eq!(*tracker.tip_id(), 0);
        assert!(tracker.contains_orphan(&2));

        // Add parent - should attach orphan
        let result = tracker.append(TestItem::new(1, 1, 0));
        assert!(matches!(result, AppendResult::Attached(2)));
        assert_eq!(*tracker.tip_id(), 2);
        assert!(!tracker.contains_orphan(&2));
        assert!(tracker.contains_unfinalized(&2));
    }

    #[test]
    fn test_orphan_chain_reattachment() {
        let finalized = TestItem::new(0, 0, 0);
        let mut tracker: ChainTracker<TestItem> = ChainTracker::new(finalized);

        // Add chain of orphans in reverse: 3 -> 2 -> 1
        tracker.append(TestItem::new(3, 3, 2));
        tracker.append(TestItem::new(2, 2, 1));

        assert!(tracker.contains_orphan(&3));
        assert!(tracker.contains_orphan(&2));

        // Add 1 - should cascade attach 2 and 3
        let result = tracker.append(TestItem::new(1, 1, 0));
        assert!(matches!(result, AppendResult::Attached(3)));

        assert!(!tracker.contains_orphan(&2));
        assert!(!tracker.contains_orphan(&3));
        assert!(tracker.contains_unfinalized(&1));
        assert!(tracker.contains_unfinalized(&2));
        assert!(tracker.contains_unfinalized(&3));
    }

    #[test]
    fn test_new_with_unfinalized() {
        let finalized = TestItem::new(0, 0, 0);
        let unfinalized = vec![
            TestItem::new(1, 1, 0),
            TestItem::new(2, 2, 1),
            TestItem::new(3, 3, 2),
        ];

        let tracker: ChainTracker<TestItem> =
            ChainTracker::new_with_unfinalized(finalized, unfinalized);

        assert_eq!(*tracker.tip_id(), 3);
        assert!(tracker.contains_unfinalized(&1));
        assert!(tracker.contains_unfinalized(&2));
        assert!(tracker.contains_unfinalized(&3));
    }

    #[test]
    fn test_canonical_chain() {
        //     0 (finalized)
        //    / \
        //   1   4
        //   |
        //   2
        //   |
        //   3
        let finalized = TestItem::new(0, 0, 0);
        let mut tracker: ChainTracker<TestItem> = ChainTracker::new(finalized);

        tracker.append(TestItem::new(1, 1, 0));
        tracker.append(TestItem::new(2, 2, 1));
        tracker.append(TestItem::new(3, 3, 2));
        tracker.append(TestItem::new(1, 4, 0)); // Fork

        assert_eq!(tracker.canonical_chain(), &[1, 2, 3]);
        assert!(tracker.is_canonical(&0));
        assert!(tracker.is_canonical(&1));
        assert!(tracker.is_canonical(&2));
        assert!(tracker.is_canonical(&3));
        assert!(!tracker.is_canonical(&4));
    }

    #[test]
    fn test_canonical_id_at_index() {
        let finalized = TestItem::new(0, 0, 0);
        let mut tracker: ChainTracker<TestItem> = ChainTracker::new(finalized);

        tracker.append(TestItem::new(1, 1, 0));
        tracker.append(TestItem::new(2, 2, 1));

        assert_eq!(tracker.canonical_id_at_index(0), Some(&0));
        assert_eq!(tracker.canonical_id_at_index(1), Some(&1));
        assert_eq!(tracker.canonical_id_at_index(2), Some(&2));
        assert_eq!(tracker.canonical_id_at_index(3), None);
    }

    #[test]
    fn test_prune_to() {
        // 0 -> 1 -> 2 -> 3
        let finalized = TestItem::new(0, 0, 0);
        let mut tracker: ChainTracker<TestItem> = ChainTracker::new(finalized);

        tracker.append(TestItem::new(1, 1, 0));
        tracker.append(TestItem::new(2, 2, 1));
        tracker.append(TestItem::new(3, 3, 2));

        let report = tracker.prune_to(2).unwrap();

        assert_eq!(report.finalized, vec![1]);
        assert!(report.pruned.is_empty());
        assert_eq!(*tracker.finalized_id(), 2);
        assert_eq!(tracker.finalized_index(), 2);
        assert!(tracker.contains_unfinalized(&3));
        assert!(!tracker.contains_unfinalized(&1));
    }

    #[test]
    fn test_prune_removes_fork() {
        //     0
        //    / \
        //   1   3
        //   |   |
        //   2   4
        //
        // Finalize 1, should prune 3 and 4
        let finalized = TestItem::new(0, 0, 0);
        let mut tracker: ChainTracker<TestItem> = ChainTracker::new(finalized);

        tracker.append(TestItem::new(1, 1, 0));
        tracker.append(TestItem::new(2, 2, 1));
        tracker.append(TestItem::new(1, 3, 0));
        tracker.append(TestItem::new(2, 4, 3));

        let report = tracker.prune_to(1).unwrap();

        assert!(report.finalized.is_empty());
        assert_eq!(report.pruned.len(), 2);
        assert!(report.pruned.contains(&3));
        assert!(report.pruned.contains(&4));

        assert_eq!(*tracker.finalized_id(), 1);
        assert!(tracker.contains_unfinalized(&2));
        assert!(!tracker.contains_unfinalized(&3));
        assert!(!tracker.contains_unfinalized(&4));
    }

    #[test]
    fn test_prune_removes_old_orphans() {
        let finalized = TestItem::new(0, 0, 0);
        let mut tracker: ChainTracker<TestItem> = ChainTracker::new(finalized);

        tracker.append(TestItem::new(1, 1, 0));
        tracker.append(TestItem::new(2, 2, 1));

        // Add orphans at different heights
        tracker.append(TestItem::new(1, 10, 99)); // height 1
        tracker.append(TestItem::new(2, 11, 99)); // height 2
        tracker.append(TestItem::new(3, 12, 99)); // height 3

        assert!(tracker.contains_orphan(&10));
        assert!(tracker.contains_orphan(&11));
        assert!(tracker.contains_orphan(&12));

        // Finalize to height 2
        let report = tracker.prune_to(2).unwrap();

        // Orphans at height 1 and 2 should be pruned
        assert!(report.pruned.contains(&10));
        assert!(report.pruned.contains(&11));
        assert!(!tracker.contains_orphan(&10));
        assert!(!tracker.contains_orphan(&11));
        assert!(tracker.contains_orphan(&12)); // height 3, kept
    }

    #[test]
    fn test_orphan_on_side_chain() {
        //        0 (finalized)
        //       / \
        //      1   3 (side chain)
        //      |   |
        //      2   4 (orphan, child of 3)
        //
        // Add orphan 4 before 3, then add 3
        let finalized = TestItem::new(0, 0, 0);
        let mut tracker: ChainTracker<TestItem> = ChainTracker::new(finalized);

        tracker.append(TestItem::new(1, 1, 0));
        tracker.append(TestItem::new(2, 2, 1));

        // Add orphan 4 (parent 3 missing)
        tracker.append(TestItem::new(2, 4, 3));
        assert!(tracker.contains_orphan(&4));

        // Add 3 (side chain) - should attach orphan 4
        tracker.append(TestItem::new(1, 3, 0));

        assert!(!tracker.contains_orphan(&4));
        assert!(tracker.contains_unfinalized(&4));

        // Tip should still be 2 (taller chain)
        assert_eq!(*tracker.tip_id(), 2);
    }
}
