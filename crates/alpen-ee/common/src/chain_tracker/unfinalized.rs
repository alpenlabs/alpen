//! Unfinalized chain tracker managing the tree of items from finalized to tips.

use std::{
    collections::{HashMap, HashSet},
    fmt::Debug,
    hash::Hash,
};

use super::{error::ChainTrackerError, item::ItemEntry};

/// An identifier paired with its index/height.
#[derive(Debug, Clone)]
pub(crate) struct IdWithIndex<Id> {
    pub id: Id,
    pub index: u64,
}

/// Result of attempting to attach an item to the tracker.
#[derive(Debug)]
pub(crate) enum AttachResult<Id> {
    /// Successfully attached, returns new best tip id
    Attached(Id),
    /// Item already exists in tracker
    Existing,
    /// Item index is below finalized, cannot attach
    BelowFinalized,
    /// Item's parent is unknown, item is an orphan
    Orphan,
}

/// Report of items affected by finalization/pruning.
#[derive(Debug)]
pub struct PruneReport<Id> {
    /// Items that became finalized (oldest to newest, excludes the new finalized item itself)
    pub finalized: Vec<Id>,
    /// Items pruned from memory (were on non-canonical forks)
    pub pruned: Vec<Id>,
}

impl<Id> PruneReport<Id> {
    fn new(finalized: Vec<Id>, pruned: Vec<Id>) -> Self {
        Self { finalized, pruned }
    }

    pub(crate) fn new_empty() -> Self {
        Self {
            finalized: Vec::new(),
            pruned: Vec::new(),
        }
    }
}

/// Tracks unfinalized items and maintains chain tips between finalized and best tip.
///
/// Manages a tree of items starting from the last finalized item, tracking all competing
/// chain tips and identifying the best (highest) tip. Also maintains the canonical chain
/// for efficient queries.
#[derive(Debug)]
pub(crate) struct UnfinalizedTracker<Id> {
    /// The last finalized item
    finalized: IdWithIndex<Id>,
    /// The current best (highest) chain tip
    best: IdWithIndex<Id>,
    /// Active chain tips mapping id to index
    tips: HashMap<Id, u64>,
    /// All tracked items mapping id to entry
    entries: HashMap<Id, ItemEntry<Id>>,
    /// Canonical chain from finalized (exclusive) to best (inclusive)
    /// canonical[0] is at index (finalized.index + 1)
    canonical: Vec<Id>,
    /// Set of ids on canonical chain for O(1) lookup
    canonical_set: HashSet<Id>,
}

impl<Id: Clone + Eq + Hash + Debug> UnfinalizedTracker<Id> {
    /// Creates a new tracker with the given finalized item as the starting point.
    pub(crate) fn new(finalized_entry: ItemEntry<Id>) -> Self {
        let id = finalized_entry.id.clone();
        let index = finalized_entry.index;

        Self {
            finalized: IdWithIndex {
                id: id.clone(),
                index,
            },
            best: IdWithIndex {
                id: id.clone(),
                index,
            },
            tips: HashMap::from([(id.clone(), index)]),
            entries: HashMap::from([(id, finalized_entry)]),
            canonical: Vec::new(),
            canonical_set: HashSet::new(),
        }
    }

    /// Attempts to attach an item to the tracker.
    ///
    /// Returns the result of the attachment attempt, updating tips, best, and canonical
    /// chain if successful.
    pub(crate) fn attach(&mut self, entry: ItemEntry<Id>) -> AttachResult<Id> {
        let id = entry.id.clone();

        // 1. Is it an existing item?
        if self.entries.contains_key(&id) {
            return AttachResult::Existing;
        }

        // 2. Is it below finalized?
        let item_index = entry.index;
        if item_index < self.finalized.index {
            return AttachResult::BelowFinalized;
        }

        let parent_id = entry.parent_id.clone();

        // 3. Does it extend an existing tip?
        if self.tips.contains_key(&parent_id) {
            self.entries.insert(id.clone(), entry);
            self.tips.remove(&parent_id);
            self.tips.insert(id.clone(), item_index);

            self.update_best_and_canonical();
            return AttachResult::Attached(self.best.id.clone());
        }

        // 4. Does it create a new tip (fork)?
        if self.entries.contains_key(&parent_id) {
            self.entries.insert(id.clone(), entry);
            self.tips.insert(id.clone(), item_index);

            self.update_best_and_canonical();
            return AttachResult::Attached(self.best.id.clone());
        }

        // Parent not known - orphan
        AttachResult::Orphan
    }

    /// Updates best tip and rebuilds canonical chain if best changed.
    fn update_best_and_canonical(&mut self) {
        let (new_best_id, new_best_index) = self.compute_best_tip();

        if new_best_id != self.best.id {
            self.best.id = new_best_id;
            self.best.index = new_best_index;
            self.rebuild_canonical();
        }
    }

    /// Finds the tip with the highest index.
    /// On ties, keeps the current best (first-seen preference).
    fn compute_best_tip(&self) -> (Id, u64) {
        self.tips.iter().fold(
            (self.best.id.clone(), self.best.index),
            |(best_id, best_index), (id, &index)| {
                if index > best_index {
                    (id.clone(), index)
                } else {
                    (best_id, best_index)
                }
            },
        )
    }

    /// Rebuilds the canonical chain by walking from best to finalized.
    fn rebuild_canonical(&mut self) {
        self.canonical.clear();
        self.canonical_set.clear();

        let mut current = self.best.id.clone();
        while current != self.finalized.id {
            self.canonical.push(current.clone());
            self.canonical_set.insert(current.clone());
            current = self
                .entries
                .get(&current)
                .expect("chain should be connected")
                .parent_id
                .clone();
        }

        self.canonical.reverse(); // Now oldest-to-newest
    }

    /// Checks if an item with the given id is tracked.
    pub(crate) fn contains(&self, id: &Id) -> bool {
        self.entries.contains_key(id)
    }

    /// Returns the finalized item's id.
    pub(crate) fn finalized_id(&self) -> &Id {
        &self.finalized.id
    }

    /// Returns the finalized item's index.
    pub(crate) fn finalized_index(&self) -> u64 {
        self.finalized.index
    }

    /// Returns the best tip's id.
    pub(crate) fn best_id(&self) -> &Id {
        &self.best.id
    }

    #[cfg(test)]
    /// Returns the best tip's index.
    pub(crate) fn best_index(&self) -> u64 {
        self.best.index
    }

    /// Returns the canonical chain from finalized (exclusive) to tip (inclusive).
    pub(crate) fn canonical_chain(&self) -> &[Id] {
        &self.canonical
    }

    /// Returns the id at the given index on the canonical chain.
    /// Returns None if index is below finalized or above tip.
    pub(crate) fn canonical_id_at_index(&self, index: u64) -> Option<&Id> {
        if index == self.finalized.index {
            Some(&self.finalized.id)
        } else if index < self.finalized.index || index > self.best.index {
            None
        } else {
            let offset = (index - self.finalized.index - 1) as usize;
            self.canonical.get(offset)
        }
    }

    /// Checks if an item is on the canonical chain.
    pub(crate) fn is_canonical(&self, id: &Id) -> bool {
        *id == self.finalized.id || self.canonical_set.contains(id)
    }

    /// Advances finalization to the given item and prunes non-canonical items.
    ///
    /// Returns a report of newly finalized items and items that were pruned.
    pub(crate) fn prune_to(
        &mut self,
        new_finalized_id: Id,
    ) -> Result<PruneReport<Id>, ChainTrackerError<Id>> {
        if new_finalized_id == self.finalized.id {
            return Ok(PruneReport::new_empty());
        }

        let Some(new_finalized_entry) = self.entries.remove(&new_finalized_id) else {
            return Err(ChainTrackerError::UnknownItem(new_finalized_id));
        };

        // Collect all items that are being finalized (walk from new finalized to old finalized)
        let finalized_count = new_finalized_entry.index - self.finalized.index;
        let mut finalized_ids = Vec::<Id>::with_capacity(finalized_count as usize);
        let mut current = new_finalized_entry.clone();

        for _ in 0..finalized_count {
            finalized_ids.push(current.id.clone());
            current = self
                .entries
                .remove(&current.parent_id)
                .expect("chain should be connected");
        }

        // Sanity check: we should have reached the old finalized
        if current.id != self.finalized.id {
            return Err(ChainTrackerError::InvalidState);
        }

        // Rebuild tracker with new finalized as root
        let mut new_tracker = Self::new(new_finalized_entry);
        let mut remaining_entries: Vec<_> = self.entries.drain().map(|(_, e)| e).collect();

        // Sort by index to attach in order
        remaining_entries.sort_by_key(|e| e.index);

        let mut pruned = Vec::new();
        for entry in remaining_entries {
            let entry_id = entry.id.clone();
            match new_tracker.attach(entry) {
                AttachResult::Attached(_) | AttachResult::Existing => {}
                AttachResult::Orphan | AttachResult::BelowFinalized => {
                    pruned.push(entry_id);
                }
            }
        }

        *self = new_tracker;

        // Reverse to get oldest-to-newest order, exclude new finalized itself
        finalized_ids.reverse();
        finalized_ids.pop(); // Remove the new finalized from the list

        Ok(PruneReport::new(finalized_ids, pruned))
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
    fn test_attach_to_finalized() {
        let finalized = make_entry(0, 0, 0);
        let mut tracker = UnfinalizedTracker::new(finalized);

        let item1 = make_entry(1, 1, 0);
        let result = tracker.attach(item1);

        assert!(matches!(result, AttachResult::Attached(_)));
        assert_eq!(*tracker.best_id(), 1);
        assert!(tracker.contains(&1));
    }

    #[test]
    fn test_attach_linear_chain() {
        let finalized = make_entry(0, 0, 0);
        let mut tracker = UnfinalizedTracker::new(finalized);

        tracker.attach(make_entry(1, 1, 0));
        tracker.attach(make_entry(2, 2, 1));
        tracker.attach(make_entry(3, 3, 2));

        assert_eq!(*tracker.best_id(), 3);
        assert_eq!(tracker.best_index(), 3);
        assert_eq!(tracker.canonical_chain(), &[1, 2, 3]);
    }

    #[test]
    fn test_attach_fork() {
        //     0 (finalized)
        //    / \
        //   1   2
        //   |
        //   3
        let finalized = make_entry(0, 0, 0);
        let mut tracker = UnfinalizedTracker::new(finalized);

        tracker.attach(make_entry(1, 1, 0));
        tracker.attach(make_entry(1, 2, 0));
        tracker.attach(make_entry(2, 3, 1));

        // Item 3 is tallest, so it should be best
        assert_eq!(*tracker.best_id(), 3);
        assert_eq!(tracker.best_index(), 2);
        assert!(tracker.contains(&1));
        assert!(tracker.contains(&2));
        assert!(tracker.contains(&3));
        assert_eq!(tracker.canonical_chain(), &[1, 3]);
    }

    #[test]
    fn test_existing_item() {
        let finalized = make_entry(0, 0, 0);
        let mut tracker = UnfinalizedTracker::new(finalized);

        let item1 = make_entry(1, 1, 0);
        tracker.attach(item1.clone());

        let result = tracker.attach(item1);
        assert!(matches!(result, AttachResult::Existing));
    }

    #[test]
    fn test_below_finalized() {
        let finalized = make_entry(5, 5, 4);
        let mut tracker = UnfinalizedTracker::new(finalized);

        let item = make_entry(3, 3, 2);
        let result = tracker.attach(item);

        assert!(matches!(result, AttachResult::BelowFinalized));
    }

    #[test]
    fn test_orphan() {
        let finalized = make_entry(0, 0, 0);
        let mut tracker = UnfinalizedTracker::new(finalized);

        // Try to attach item 2 without item 1
        let item2 = make_entry(2, 2, 1);
        let result = tracker.attach(item2);

        assert!(matches!(result, AttachResult::Orphan));
    }

    #[test]
    fn test_best_tip_first_seen_preference() {
        //     0 (finalized)
        //    /|\
        //   1 2 3
        let finalized = make_entry(0, 0, 0);
        let mut tracker = UnfinalizedTracker::new(finalized);

        tracker.attach(make_entry(1, 1, 0));
        tracker.attach(make_entry(1, 2, 0));
        tracker.attach(make_entry(1, 3, 0));

        // First one attached (1) should remain best when all same height
        assert_eq!(*tracker.best_id(), 1);
        assert_eq!(tracker.best_index(), 1);
    }

    #[test]
    fn test_canonical_id_at_index() {
        let finalized = make_entry(0, 0, 0);
        let mut tracker = UnfinalizedTracker::new(finalized);

        tracker.attach(make_entry(1, 1, 0));
        tracker.attach(make_entry(2, 2, 1));
        tracker.attach(make_entry(3, 3, 2));

        assert_eq!(tracker.canonical_id_at_index(0), Some(&0));
        assert_eq!(tracker.canonical_id_at_index(1), Some(&1));
        assert_eq!(tracker.canonical_id_at_index(2), Some(&2));
        assert_eq!(tracker.canonical_id_at_index(3), Some(&3));
        assert_eq!(tracker.canonical_id_at_index(4), None);
    }

    #[test]
    fn test_is_canonical() {
        //     0 (finalized)
        //    / \
        //   1   2
        //   |
        //   3
        let finalized = make_entry(0, 0, 0);
        let mut tracker = UnfinalizedTracker::new(finalized);

        tracker.attach(make_entry(1, 1, 0));
        tracker.attach(make_entry(1, 2, 0));
        tracker.attach(make_entry(2, 3, 1));

        assert!(tracker.is_canonical(&0));
        assert!(tracker.is_canonical(&1));
        assert!(!tracker.is_canonical(&2)); // Side chain
        assert!(tracker.is_canonical(&3));
    }

    #[test]
    fn test_prune_linear_chain() {
        // 0 -> 1 -> 2 -> 3
        // Finalize up to 2
        let finalized = make_entry(0, 0, 0);
        let mut tracker = UnfinalizedTracker::new(finalized);

        tracker.attach(make_entry(1, 1, 0));
        tracker.attach(make_entry(2, 2, 1));
        tracker.attach(make_entry(3, 3, 2));

        let report = tracker.prune_to(2).unwrap();

        // Items 1 should be finalized (0 was already finalized, 2 is new finalized)
        assert_eq!(report.finalized, vec![1]);
        assert!(report.pruned.is_empty());

        assert_eq!(*tracker.finalized_id(), 2);
        assert!(tracker.contains(&3));
        assert!(!tracker.contains(&1));

        // Verify canonical chain is correct after prune
        assert_eq!(tracker.canonical_chain(), &[3]);
        assert!(tracker.is_canonical(&2)); // finalized
        assert!(tracker.is_canonical(&3));
        assert_eq!(tracker.canonical_id_at_index(2), Some(&2));
        assert_eq!(tracker.canonical_id_at_index(3), Some(&3));
    }

    #[test]
    fn test_prune_with_fork() {
        //     0
        //    / \
        //   1   2
        //   |   |
        //   3   4
        //
        // Finalize 2, should remove 1 and 3
        let finalized = make_entry(0, 0, 0);
        let mut tracker = UnfinalizedTracker::new(finalized);

        tracker.attach(make_entry(1, 1, 0));
        tracker.attach(make_entry(1, 2, 0));
        tracker.attach(make_entry(2, 3, 1));
        tracker.attach(make_entry(2, 4, 2));

        let report = tracker.prune_to(2).unwrap();

        assert!(report.finalized.is_empty());
        assert_eq!(report.pruned.len(), 2);
        assert!(report.pruned.contains(&1));
        assert!(report.pruned.contains(&3));

        assert_eq!(*tracker.finalized_id(), 2);
        assert!(tracker.contains(&4));
        assert!(!tracker.contains(&1));
        assert!(!tracker.contains(&3));
    }

    #[test]
    fn test_prune_noop() {
        let finalized = make_entry(0, 0, 0);
        let mut tracker = UnfinalizedTracker::new(finalized);

        tracker.attach(make_entry(1, 1, 0));

        let report = tracker.prune_to(0).unwrap();

        assert!(report.finalized.is_empty());
        assert!(report.pruned.is_empty());
        assert_eq!(*tracker.finalized_id(), 0);
        assert!(tracker.contains(&1));
    }

    #[test]
    fn test_prune_to_tip() {
        // 0 -> 1 -> 2
        // Finalize to 2 (the tip), leaving no unfinalized
        let finalized = make_entry(0, 0, 0);
        let mut tracker = UnfinalizedTracker::new(finalized);

        tracker.attach(make_entry(1, 1, 0));
        tracker.attach(make_entry(2, 2, 1));

        let report = tracker.prune_to(2).unwrap();

        assert_eq!(report.finalized, vec![1]);
        assert!(report.pruned.is_empty());

        assert_eq!(*tracker.finalized_id(), 2);
        assert_eq!(*tracker.best_id(), 2);
        assert!(tracker.canonical_chain().is_empty());
        assert!(tracker.is_canonical(&2));
        assert_eq!(tracker.canonical_id_at_index(2), Some(&2));
    }

    #[test]
    fn test_prune_unknown_item() {
        let finalized = make_entry(0, 0, 0);
        let mut tracker = UnfinalizedTracker::new(finalized);

        tracker.attach(make_entry(1, 1, 0));

        let result = tracker.prune_to(99);
        assert!(matches!(result, Err(ChainTrackerError::UnknownItem(99))));
    }

    #[test]
    fn test_canonical_updates_on_reorg() {
        //     0 (finalized)
        //    / \
        //   1   2
        //   |
        //   3
        //   |
        //   4  (eventually taller chain wins)
        let finalized = make_entry(0, 0, 0);
        let mut tracker = UnfinalizedTracker::new(finalized);

        tracker.attach(make_entry(1, 1, 0));
        assert_eq!(tracker.canonical_chain(), &[1]);

        tracker.attach(make_entry(1, 2, 0));
        // 1 was first, stays best
        assert_eq!(tracker.canonical_chain(), &[1]);

        tracker.attach(make_entry(2, 3, 1));
        assert_eq!(tracker.canonical_chain(), &[1, 3]);

        // Now extend chain 2 to be taller
        tracker.attach(make_entry(2, 4, 2));
        // Still 1->3 is taller (height 2)
        assert_eq!(tracker.canonical_chain(), &[1, 3]);

        tracker.attach(make_entry(3, 5, 4));
        // Now 2->4->5 is taller (height 3)
        assert_eq!(tracker.canonical_chain(), &[2, 4, 5]);
    }
}
