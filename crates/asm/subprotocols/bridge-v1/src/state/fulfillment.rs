//! Withdrawal Fulfillment Tracking
//!
//! This module contains types and tables for tracking fulfilled withdrawal assignments.
//! Once an operator successfully fulfills a withdrawal, a fulfillment entry is created
//! to record which operator completed which deposit's withdrawal.

use borsh::{BorshDeserialize, BorshSerialize};
use strata_bridge_types::OperatorIdx;
use strata_primitives::sorted_vec::SortedVec;

/// Entry recording a fulfilled withdrawal assignment.
///
/// This represents a completed withdrawal where an operator has successfully
/// fronted the withdrawal transaction for a specific deposit.
#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct FulfillmentEntry {
    /// Index of the deposit that was fulfilled.
    deposit_idx: u32,

    /// Index of the operator who fulfilled the withdrawal.
    operator_idx: OperatorIdx,
}

impl FulfillmentEntry {
    /// Creates a new fulfillment entry.
    pub fn new(deposit_idx: u32, operator_idx: OperatorIdx) -> Self {
        Self {
            deposit_idx,
            operator_idx,
        }
    }

    /// Returns the deposit index.
    pub fn deposit_idx(&self) -> u32 {
        self.deposit_idx
    }

    /// Returns the operator index.
    pub fn operator_idx(&self) -> OperatorIdx {
        self.operator_idx
    }
}

impl PartialOrd for FulfillmentEntry {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for FulfillmentEntry {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.deposit_idx.cmp(&other.deposit_idx)
    }
}

/// Table for managing fulfilled withdrawal assignments.
///
/// This table maintains records of all fulfilled withdrawals, providing
/// efficient insertion, lookup, and removal operations. The table maintains
/// sorted order by deposit index for binary search efficiency.
///
/// # Ordering Invariant
///
/// The fulfillments vector **MUST** remain sorted by deposit index at all times.
/// This invariant enables O(log n) lookup operations via binary search.
#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct FulfillmentTable {
    /// Vector of fulfillment entries, sorted by deposit index.
    ///
    /// **Invariant**: MUST be sorted by `FulfillmentEntry::deposit_idx` field.
    fulfillments: SortedVec<FulfillmentEntry>,
}

impl FulfillmentTable {
    /// Creates a new empty fulfillment table.
    pub fn new() -> Self {
        Self {
            fulfillments: SortedVec::new_empty(),
        }
    }

    /// Returns the number of fulfillments in the table.
    pub fn len(&self) -> usize {
        self.fulfillments.len()
    }

    /// Returns whether the fulfillment table is empty.
    pub fn is_empty(&self) -> bool {
        self.fulfillments.is_empty()
    }

    /// Returns a slice of all fulfillment entries.
    pub fn fulfillments(&self) -> &[FulfillmentEntry] {
        self.fulfillments.as_slice()
    }

    /// Adds a new fulfillment entry to the table.
    ///
    /// # Panics
    ///
    /// Panics if a fulfillment with the given deposit index already exists.
    pub fn add(&mut self, entry: FulfillmentEntry) {
        // Check if entry already exists
        if self.get(entry.deposit_idx()).is_some() {
            panic!(
                "Fulfillment with deposit index {} already exists",
                entry.deposit_idx()
            );
        }

        // SortedVec handles the insertion and maintains order
        self.fulfillments.insert(entry);
    }

    /// Removes a fulfillment entry by deposit index.
    ///
    /// # Returns
    ///
    /// - `Some(FulfillmentEntry)` if the fulfillment was found and removed
    /// - `None` if no fulfillment with the given deposit index exists
    pub fn remove(&mut self, deposit_idx: u32) -> Option<FulfillmentEntry> {
        // Find the fulfillment first
        let fulfillment = self.get(deposit_idx)?.clone();

        // Remove it using SortedVec's remove method
        if self.fulfillments.remove(&fulfillment) {
            Some(fulfillment)
        } else {
            None
        }
    }

    /// Checks if a fulfillment exists for the given deposit index.
    pub fn contains(&self, deposit_idx: u32) -> bool {
        self.get(deposit_idx).is_some()
    }

    /// Gets a fulfillment entry by deposit index using binary search.
    ///
    /// # Returns
    ///
    /// - `Some(&FulfillmentEntry)` if the fulfillment exists
    /// - `None` if no fulfillment with the given deposit index is found
    pub fn get(&self, deposit_idx: u32) -> Option<&FulfillmentEntry> {
        self.fulfillments
            .as_slice()
            .binary_search_by_key(&deposit_idx, |entry| entry.deposit_idx())
            .ok()
            .map(|i| &self.fulfillments.as_slice()[i])
    }
}

impl Default for FulfillmentTable {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fulfillment_entry_creation() {
        let entry = FulfillmentEntry::new(42, 7);
        assert_eq!(entry.deposit_idx(), 42);
        assert_eq!(entry.operator_idx(), 7);
    }

    #[test]
    fn test_fulfillment_table_basic_operations() {
        let mut table = FulfillmentTable::new();
        assert!(table.is_empty());
        assert_eq!(table.len(), 0);

        let entry1 = FulfillmentEntry::new(10, 1);
        let entry2 = FulfillmentEntry::new(20, 2);

        // Add fulfillments
        table.add(entry1.clone());
        assert_eq!(table.len(), 1);
        assert!(!table.is_empty());

        table.add(entry2.clone());
        assert_eq!(table.len(), 2);

        // Check contains
        assert!(table.contains(10));
        assert!(table.contains(20));
        assert!(!table.contains(30));

        // Get fulfillment
        let retrieved = table.get(10);
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().deposit_idx(), 10);
        assert_eq!(retrieved.unwrap().operator_idx(), 1);

        // Remove fulfillment
        let removed = table.remove(10);
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().deposit_idx(), 10);
        assert_eq!(table.len(), 1);
        assert!(!table.contains(10));

        // Remove non-existent
        let removed = table.remove(999);
        assert!(removed.is_none());
        assert_eq!(table.len(), 1);
    }

    #[test]
    fn test_fulfillment_table_get_nonexistent() {
        let table = FulfillmentTable::new();
        assert!(table.get(42).is_none());
    }
}
