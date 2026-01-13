//! State diff types for the Alpen Reth node.
//!
//! This crate provides state diff types for encoding EE state changes,
//! organized into two tiers:
//!
//! # Architecture
//!
//! ```text
//! BundleState → BlockStateDiff → stored per block (DB)
//!                                       ↓
//! BlockStateDiff[n..m] → BatchBuilder → BatchStateDiff → DA (through Codec)
//!                                              ↓
//!                                 StateReconstructor.apply_diff()
//! ```
//!
//! # Modules
//!
//! - [`block`]: Per-block diff types stored in DB (preserves original values)
//! - [`batch`]: DA-optimized batch diff types (compact, no originals)
//! - `reconstruct`: State reconstruction from diffs (see [`StateReconstructor`])
//!
//! # Key Types
//!
//! | Type | Module | Purpose |
//! |------|--------|---------|
//! | [`BlockStateDiff`] | `block` | Per-block diff for DB storage |
//! | [`BatchStateDiff`] | `batch` | Aggregated diff for DA |
//! | [`BatchBuilder`] | `batch` | Aggregates blocks with revert detection |
//! | [`StateReconstructor`] | `reconstruct` | Applies diffs to rebuild state |

#![cfg_attr(not(test), warn(unused_crate_dependencies))]

// serde_json dev-dep is only used in serde tests (which is behind serde feature itself)
#[cfg(test)]
use serde_json as _;

pub mod batch;
pub mod block;
mod codec;
mod reconstruct;
#[cfg(feature = "serde")]
mod serde_impl;

// Re-export main types at crate level for convenience
pub use batch::{AccountChange, AccountDiff, BatchBuilder, BatchStateDiff, StorageDiff};
pub use block::{AccountSnapshot, BlockAccountChange, BlockStateDiff, BlockStorageDiff};
pub use reconstruct::{ReconstructError, StateReconstructor};
#[cfg(feature = "serde")]
pub use serde_impl::{AccountChangeSerde, AccountDiffSerde, BatchStateDiffSerde};
