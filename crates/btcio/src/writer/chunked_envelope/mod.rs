//! Chunked envelope sub-module: one commit transaction funding N reveal
//! transactions, each carrying opaque witness data.

pub(crate) mod builder;
mod context;
mod handle;
mod reader;
mod signer;

pub use handle::{create_chunked_envelope_task, ChunkedEnvelopeHandle};
pub use reader::{extract_chunk_envelope_payload, ExtractRevealError};
