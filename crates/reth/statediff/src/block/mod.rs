//! Per-block state diff types for DB storage.
//!
//! These types preserve original values to enable proper batch aggregation
//! with revert detection across multiple blocks.

mod account;
mod diff;
mod storage;

pub use account::{AccountSnapshot, BlockAccountChange};
pub use diff::BlockStateChanges;
pub use storage::BlockStorageDiff;
