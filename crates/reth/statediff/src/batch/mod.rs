//! DA-optimized batch state diff types.
//!
//! These types are compact representations suitable for posting to the DA layer.
//! They do not store original values since they represent the net change over
//! a batch of blocks.

mod account;
mod builder;
mod diff;
mod storage;

pub use account::{AccountChange, AccountDiff};
pub use builder::BatchBuilder;
pub use diff::BatchStateDiff;
pub use storage::StorageDiff;
