//! Data availability provider implementations for the Alpen EE.
//!
//! Contains [`StateDiffBlobProvider`], the concrete [`DaBlobSource`](alpen_ee_common::DaBlobSource)
//! implementation that builds encoded DA blobs from per-block Reth state diffs,
//! and [`ChunkedEnvelopeDaProvider`], the
//! [`BatchDaProvider`](alpen_ee_common::BatchDaProvider) implementation that
//! splits DA blobs into chunks and submits them as chunked envelope entries
//! for L1 inscription.

mod blob_provider;
mod envelope_provider;

pub use blob_provider::StateDiffBlobProvider;
pub use envelope_provider::ChunkedEnvelopeDaProvider;
