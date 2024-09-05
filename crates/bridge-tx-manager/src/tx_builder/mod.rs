//! Build bitcoin scripts.
//!
//! Handles creation of bitcoin scripts via `bitcoin-rs`. Provides high-level APIs to get
//! fully-formed bridge-related scripts.

pub mod deposit;
pub mod withdrawal;

// Re-exports
pub use deposit::*;
pub use withdrawal::*;
