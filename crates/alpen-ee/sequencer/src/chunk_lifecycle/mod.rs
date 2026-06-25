//! Chunk proof lifecycle manager.
//!
//! Drives every chunk's proof from `Sealed` to `ProofPending` to `ProofReady`, independently of the
//! batch/acct lifecycle. Work discovery is storage-driven: storage indexes sealed chunks for new
//! proof submission and proof-pending chunks for status polling. The in-memory state is only a
//! fairness cursor for paged queries, so reorged chunks are discovered from storage instead of
//! being hidden behind a cached floor.

mod ctx;
mod lifecycle;
mod state;
mod task;

pub use task::chunk_lifecycle_task;
