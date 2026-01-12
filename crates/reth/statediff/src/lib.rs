//! State diff for the reth node.
//!
//! This crate provides DA-optimized state diff types for encoding EE state changes.
//! Key features:
//!
//! - Deterministic encoding via sorted `BTreeMap`
//! - Delta encoding for nonces (u8 increment instead of full u64)
//! - Distinction between Created vs Updated accounts
//! - Direct conversion from `BundleState` for efficient batch processing

#![cfg_attr(not(test), warn(unused_crate_dependencies))]

// serde_json dev-dep is only used in serde_types tests (which is behind serde feature itself)
#[cfg(test)]
use serde_json as _;

mod account;
mod builder;
mod codec;
mod diff;
#[cfg(feature = "serde")]
mod serde_types;
mod state;
mod storage;

// Re-export main types at crate level
pub use account::{DaAccountChange, DaAccountDiff};
pub use builder::DaEeStateDiffBuilder;
pub use diff::DaEeStateDiff;
#[cfg(feature = "serde")]
pub use serde_types::{DaAccountChangeSerde, DaAccountDiffSerde, DaEeStateDiffSerde};
pub use state::ReconstructedState;
pub use storage::DaAccountStorageDiff;
