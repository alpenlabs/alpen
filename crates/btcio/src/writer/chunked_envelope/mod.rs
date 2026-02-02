//! Chunked envelope sub-module: one commit transaction funding N reveal
//! transactions, each carrying opaque witness data.

pub(crate) mod builder;
mod signer;
mod task;

pub use task::{start_chunked_envelope_task, ChunkedEnvelopeHandle};
