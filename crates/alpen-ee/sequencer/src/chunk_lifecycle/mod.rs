//! Chunk proof lifecycle manager.
//!
//! Drives every chunk's proof from `Sealed` to a terminal `ProofReady`/`ProofFailed`, independently
//! of the batch/acct lifecycle. The task is stateless: each tick it derives a working floor from
//! batch status and reconciles the chunks above it by their `ChunkStatus`. The floor works because
//! a batch only reaches `ProofPending` once the acct gate has confirmed all of its chunks are
//! proven, so any chunk belonging to a batch at that point is already done and can be skipped.
//! Keeping no persisted or in-memory cursor makes it restart-safe by construction, mirroring how
//! `batch_lifecycle` recovers from per-entity status alone.

mod ctx;
mod lifecycle;
mod state;
mod task;

pub use task::chunk_lifecycle_task;
