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

mod account;
mod builder;
mod codec;
mod diff;
mod state;
mod storage;

// Re-export main types at crate level
pub use account::{DaAccountChange, DaAccountDiff};
pub use builder::DaEeStateDiffBuilder;
pub use diff::{DaEeStateDiff, DaEeStateDiffSerde};
pub use state::ReconstructedState;
pub use storage::DaAccountStorageDiff;
