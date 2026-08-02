//! Chunk proof lifecycle manager.
//!
//! Drives every chunk's proof from `Sealed` to `ProofPending` to `ProofReady`, independently of the
//! batch/acct lifecycle. Work discovery is storage-driven: storage indexes sealed chunks for new
//! proof submission and proof-pending chunks for status polling. The in-memory state is only a
//! fairness cursor for paged queries plus edge-trigger bookkeeping for failure alerts, so reorged
//! chunks are discovered from storage instead of being hidden behind a cached floor.
//!
//! Chunk proving starts as soon as the owning batch seals, which is when the batch row appears in
//! storage. A chunk whose batch row is absent is deferred, because a missing row cannot be
//! distinguished from a batch reverted by a reorg whose chunk cleanup has not run yet. The win over
//! the coupled lifecycle is that chunk proving no longer waits for the batch's L1 data
//! availability; it only waits for the seal.

mod ctx;
mod lifecycle;
mod state;
mod task;

pub use task::chunk_lifecycle_task;
