//! Chunked envelope sub-module: one commit transaction funding N reveal
//! transactions, each carrying opaque witness data.

pub(crate) mod builder;
pub(crate) mod commit_op_return;
mod commit_phase;
mod context;
mod handle;
mod signer;

pub use builder::{build_chunked_envelope_txs, ChunkedEnvelopeTxs};
pub use commit_phase::{CommitPhaseClaim, CommitPhaseLatch};
pub use handle::{create_chunked_envelope_task, ChunkedEnvelopeHandle};
