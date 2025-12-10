//! Support types for OL state management.
//!
//! This crate provides utilities for working with OL state, including
//! write batching and write tracking for efficient state updates.

mod indexer_layer;
mod write_batch;
mod write_tracking_layer;

pub use indexer_layer::{
    AccumulatorWrites, IndexerAccountStateMut, IndexerSnarkAccountStateMut, IndexerState,
    InboxMessageWrite, ManifestWrite,
};
pub use write_batch::{LedgerWriteBatch, WriteBatch};
pub use write_tracking_layer::WriteTrackingState;
