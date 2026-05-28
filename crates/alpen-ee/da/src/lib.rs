//! Data availability for the Alpen EE.
//!
//! Two feature-gated halves share this crate:
//!
//! - **`runtime`** (default) — producer-side blob assembly and chunked envelope inscription:
//!   [`StateDiffBlobProvider`] (a [`DaBlobSource`](alpen_ee_common::DaBlobSource) implementation
//!   that builds encoded DA blobs from per-block Reth state diffs) and
//!   [`ChunkedEnvelopeDaProvider`] (a [`BatchDaProvider`](alpen_ee_common::BatchDaProvider)
//!   implementation that splits DA blobs into chunks and submits them as chunked envelope entries
//!   for L1 inscription).
//!
//! - **`verification`** — proof-side DA witness verifier consumed by `strata-proofimpl-alpen-acct`.
//!   Reassembles a posted blob from witnessed commit/reveal transactions, checks magic / version,
//!   and ties the result to the active update's public parameters and chunk transitions. Pulls in
//!   the Reth EVM execution stack via `strata-evm-ee`.

#[cfg(feature = "runtime")]
mod blob_provider;
#[cfg(feature = "runtime")]
mod chunking;
#[cfg(feature = "runtime")]
mod envelope_provider;

#[cfg(feature = "verification")]
pub mod verification;

#[cfg(feature = "runtime")]
pub use blob_provider::StateDiffBlobProvider;
#[cfg(feature = "runtime")]
pub use chunking::prepare_da_chunks;
#[cfg(feature = "runtime")]
pub use envelope_provider::ChunkedEnvelopeDaProvider;
