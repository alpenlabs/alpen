//! Chain item traits and types for generic chain tracking.

use std::{fmt::Debug, hash::Hash};

/// Lightweight entry for internal chain tracking.
///
/// Contains only the minimal information needed to track chain structure:
/// index (height), id (hash), and parent reference.
#[derive(Debug, Clone)]
pub struct ItemEntry<Id> {
    /// The index/height of this item in the chain
    pub index: u64,
    /// Unique identifier for this item
    pub id: Id,
    /// Identifier of the parent item this builds on
    pub parent_id: Id,
}

/// Trait for items that can be tracked in a chain.
///
/// Implementers must provide index, id, and parent_id accessors.
/// The `Id` type must be cloneable, hashable, and comparable for equality.
pub trait ChainItem {
    /// The type used to identify items (e.g., block hash)
    type Id: Clone + Eq + Hash + Debug;

    /// Returns the index/height of this item in the chain
    fn index(&self) -> u64;

    /// Returns the unique identifier of this item
    fn id(&self) -> Self::Id;

    /// Returns the identifier of the parent item
    fn parent_id(&self) -> Self::Id;

    /// Converts this item to a lightweight entry for tracking
    fn as_entry(&self) -> ItemEntry<Self::Id> {
        ItemEntry {
            index: self.index(),
            id: self.id(),
            parent_id: self.parent_id(),
        }
    }
}
