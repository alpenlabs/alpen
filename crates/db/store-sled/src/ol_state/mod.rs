//! OL state database implementation for Sled.
//!
//! This module provides persistent storage for OL execution state, including:
//! - Write batches for state reconstruction
//! - Finalized state snapshots
//! - ASM manifest MMR entries
//! - Snark account inbox messages

mod db;
mod schemas;

pub use db::OLStateDBSled;
