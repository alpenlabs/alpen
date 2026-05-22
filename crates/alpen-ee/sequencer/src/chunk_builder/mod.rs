//! Chunk builder — creates chunks from batch builder events.
//!
//! Chunking is a prover optimization: breaking batches into smaller
//! provable units. The chunk builder consumes
//! [`BatchBuilderEvent`](crate::batch_builder::BatchBuilderEvent)s from the batch builder via the
//! service framework's message loop and creates chunks according to a configurable sealing policy,
//! with batch boundaries as a hard constraint (chunks must not cross
//! batch boundaries).
//!
//! The chunk builder receives a validated, reorg-safe, ordered stream
//! of events from the batch builder rather than watching the block
//! stream directly. This guarantees it never runs ahead of the batch
//! builder and inherits reorg handling for free.
//!
//! # Startup sequence
//!
//! 1. `cleanup_orphaned_chunks` — revert chunks past the last complete batch boundary. Chunks from
//!    incomplete batches are discarded because the sealing policy cannot guarantee the same block
//!    range after restart.
//! 2. `repair_batch_linkage` — if the latest chunk's batch exists but was never linked (crash after
//!    sealing all chunks but before the boundary was processed), reconstruct the linkage from chunk
//!    `batch_idx` fields.
//! 3. [`init_chunk_builder_state`] — load state from storage. After steps 1–2 the latest chunk's
//!    batch is always linked.
//! 4. Service launch — backfill of unchunked batches runs in `on_launch` via
//!    `recovery::enqueue_backfill`.

mod handlers;
mod recovery;
mod service;
mod state;
#[cfg(test)]
mod tests;

pub use recovery::{cleanup_orphaned_chunks, repair_batch_linkage};
pub use service::{
    create_chunk_builder_state, ChunkBuilderService, ChunkBuilderServiceState, ChunkBuilderStatus,
};
pub use state::{init_chunk_builder_state, ChunkBuilderState};
