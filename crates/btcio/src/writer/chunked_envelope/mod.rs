//! Chunked envelope publication for large DA payloads.
//!
//! This module handles splitting large payloads into multiple chunks and publishing them
//! to Bitcoin using a batched commit/reveal transaction pattern:
//!
//! - **1 commit transaction** with N outputs (one per chunk)
//! - **N reveal transactions**, each spending one commit output
//!
//! Each chunk is inscribed using a taproot script-path spend with the chunk data
//! embedded in an `OP_FALSE OP_IF` envelope.
//!
//! The chunks are linked via `prev_chunk_wtxid` stored in the OP_RETURN output,
//! enabling recovery scenarios without affecting the taproot address derivation.
//!
//! # State Machine
//!
//! The module includes a state machine ([`ChunkedPublishingState`]) for tracking
//! publication progress with per-chunk status and retry logic:
//!
//! ```text
//! Pending → Submitted → Published → Confirmed
//!    ↓          ↓           ↓
//!  retry      retry       retry
//!    ↓          ↓           ↓
//!  Failed    Failed      Failed (after max retries)
//! ```

pub(crate) mod builder;
mod error;
mod handle;
mod op_return;
mod types;
mod watcher;

pub use error::ChunkedEnvelopeError;
pub use handle::ChunkedEnvelopeHandle;
pub use op_return::RawOpReturnBuilder;
pub use types::{
    ChunkPublishStatus, ChunkedEnvelopeHeader, ChunkedPayloadIntent, ChunkedPublishingState,
    ChunkedSubmissionResult, DaBlobStatus, DaStatus, DEFAULT_MAX_RETRIES, MAX_CHUNKS,
    MAX_CHUNK_PAYLOAD, MAX_PAYLOAD_SIZE,
};
