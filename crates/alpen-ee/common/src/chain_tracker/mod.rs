//! Generic chain tracker for managing chains of block-like items.
//!
//! This module provides a reusable chain tracking infrastructure that can be used
//! for any chain of items where:
//! - Each item has an incrementing index (height)
//! - Each item has a unique identifier
//! - Each item references its parent's identifier
//!
//! The tracker manages:
//! - **Finalized items**: Items that are considered permanent and won't reorg
//! - **Unfinalized items**: Items extending from finalized that may still reorg
//! - **Orphan items**: Items whose parent is not yet known
//!
//! # Usage
//!
//! ```ignore
//! use alpen_ee_common::chain_tracker::{ChainItem, ChainTracker, AppendResult};
//!
//! // Implement ChainItem for your type
//! impl ChainItem for MyBlock {
//!     type Id = Hash;
//!
//!     fn index(&self) -> u64 { self.height }
//!     fn id(&self) -> Hash { self.hash }
//!     fn parent_id(&self) -> Hash { self.parent_hash }
//! }
//!
//! // Create tracker with finalized block
//! let mut tracker = ChainTracker::new(finalized_block);
//!
//! // Append new blocks
//! match tracker.append(new_block) {
//!     AppendResult::Attached(new_tip) => { /* tip may have changed */ }
//!     AppendResult::Orphaned => { /* block tracked, waiting for parent */ }
//!     AppendResult::AlreadyExists => { /* duplicate */ }
//!     AppendResult::BelowFinalized => { /* too old */ }
//! }
//!
//! // Query canonical chain
//! let canonical = tracker.canonical_chain();
//! let block_at_height = tracker.canonical_id_at_index(5);
//! let is_on_main_chain = tracker.is_canonical(&some_id);
//!
//! // Advance finalization
//! let report = tracker.prune_to(new_finalized_id)?;
//! ```

mod error;
mod item;
mod orphan;
mod tracker;
mod unfinalized;

pub use error::ChainTrackerError;
pub use item::{ChainItem, ItemEntry};
pub use tracker::{AppendResult, ChainTracker};
pub use unfinalized::PruneReport;
