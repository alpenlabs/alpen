//! Support types for OL state management.
//!
//! This crate provides utilities for working with OL state, including
//! write batching and write tracking for efficient state updates.

mod batch_diff_layer;
mod indexer_layer;
mod write_batch;
mod write_tracking_layer;

#[cfg(test)]
mod test_utils;

#[cfg(test)]
mod tests;

pub use batch_diff_layer::BatchDiffState;
pub use indexer_layer::{
    AccumulatorWrites, InboxMessageWrite, IndexerAccountStateMut, IndexerSnarkAccountStateMut,
    IndexerState, ManifestWrite,
};
pub use write_batch::{LedgerWriteBatch, WriteBatch};
pub use write_tracking_layer::WriteTrackingState;
