//! Common sync block header trait for both L2 and OL blocks.
//! This trait provides the minimal interface needed by sync/consensus components
//! to track blocks without depending on specific block implementations.

use std::{fmt::Debug, hash::Hash};

/// Trait for block headers that can be tracked by sync/consensus components.
pub trait SyncBlockHeader: Clone + Send + Sync {
    /// The type of block identifier used.
    type BlockId: Copy + Clone + Eq + Hash + Debug + Send + Sync;

    /// Returns the slot (height) of this block.
    fn slot(&self) -> u64;

    /// Returns the parent block identifier.
    fn parent(&self) -> &Self::BlockId;
}