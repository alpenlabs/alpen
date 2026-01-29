//! State diff types for the Alpen Reth node.
//!
//! This crate provides state diff types for encoding EE state changes,
//! organized into two tiers:
//!
//! # Architecture
//!
//! ```text
//! BundleState → BlockStateChanges → stored per block (DB)
//!                                       ↓
//! BlockStateChanges[n..m] → BatchBuilder → BatchStateDiff → DA (through Codec)
//!                                              ↓
//!                                 StateReconstructor.apply_diff()
//! ```
//!
//! # Modules
//!
//! - [`block`]: Per-block changes types stored in DB (preserves original values)
//! - [`batch`]: DA-optimized batch diff types (compact, no originals), actually posted on-chain
//! - `reconstruct`: State reconstruction from diffs (see [`StateReconstructor`]), currently
//!   experimental and used only in tests - will be adjusted later (for syncing from diffs).
//!
//! # Key Types
//!
//! | Type | Module | Purpose |
//! |------|--------|---------|
//! | [`BlockStateChanges`] | `block` | Per-block changes for DB storage |
//! | [`BatchStateDiff`] | `batch` | Aggregated diff for DA |
//! | [`BatchBuilder`] | `batch` | Aggregates blocks with revert detection |
//! | [`StateReconstructor`] | `reconstruct` | Applies diffs to rebuild state |
//!
//! # Features
//!
//! ## `serde`
//!
//! Enables JSON-serializable wrapper types for RPC responses:
//!
//! | Type | Wraps | Purpose |
//! |------|-------|---------|
//! | [`BatchStateDiffSerde`] | [`BatchStateDiff`] | Full batch diff with accounts and storage |
//! | [`AccountChangeSerde`] | [`AccountChange`] | Created/Updated/Deleted enum |
//! | [`AccountDiffSerde`] | [`AccountDiff`] | Balance, nonce delta, code hash |
//!
//! These types flatten the DA framework primitives into simple JSON fields
//! (e.g., `nonce_delta: i32` instead of `DaCounter<CtrU64BySignedVarint>`).

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
pub use block::{AccountSnapshot, BlockAccountChange, BlockStateChanges, BlockStorageDiff};
pub use reconstruct::{ReconstructError, StateReconstructor};
#[cfg(feature = "serde")]
pub use serde_impl::{AccountChangeSerde, AccountDiffSerde, BatchStateDiffSerde};
