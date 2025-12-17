//! Support types for OL state management.
//!
//! This crate provides utilities for working with OL state, including
//! write batching and write tracking for efficient state updates.

// Re-export from state-types for convenience
pub use strata_ol_state_types::{LedgerWriteBatch, SerialMap, WriteBatch};
pub use write_tracking_layer::WriteTrackingState;

mod batch_diff_layer;
mod index_types;
mod indexer_layer;
mod write_tracking_layer;

#[cfg(test)]
mod test_utils;

#[cfg(test)]
mod tests;

pub use batch_diff_layer::*;
pub use index_types::*;
pub use indexer_layer::*;
