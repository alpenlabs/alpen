//! Sequencer-side data availability providers for the Alpen EE.
//!
//! Builds DA blobs from Reth state diffs ([`StateDiffBlobProvider`]) and posts
//! them to L1 as chunked-envelope inscriptions ([`ChunkedEnvelopeDaProvider`]).
//! Host-only: depends on btcio, the Bitcoin RPC client, and EE node storage, so
//! it is never linked into proof/guest builds.

mod blob_provider;
mod blob_source;
mod chunking;
mod envelope_provider;

pub use blob_provider::StateDiffBlobProvider;
pub use blob_source::DaBlobSource;
pub use chunking::{prepare_da_chunks, DEFAULT_MAX_CHUNK_PAYLOAD};
pub use envelope_provider::{ChunkedEnvelopeDaProvider, L1BlockReader};
