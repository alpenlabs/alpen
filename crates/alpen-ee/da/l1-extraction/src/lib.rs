//! EE DA extraction helpers for bounded L1 ranges.
//!
//! This crate is a stateless extractor for verifier-sized ranges.
//!
//! The intended verifier flow is fetch a bounded L1 range, scan it for
//! commit/reveal envelopes, and reassemble the discovered DA blobs.
//!
//! Scanning authenticates reveals against one sequencer key. Key rotation is not
//! modeled in this crate; callers scanning across rotations must split ranges by
//! key epoch or provide a higher-level key schedule.

mod fetch;
mod reassemble;
mod scan;

#[cfg(test)]
mod test_utils;

pub use fetch::{
    fetch_range, FetchError, FetchRangeError, FetchReader, FetchRetryPolicy, L1BlockData,
};
pub use reassemble::{reassemble_da_blobs, ReassembleError};
pub use scan::{EnvelopeScanner, EnvelopeScannerConfig, ParsedEnvelope, ScanError, ScanOutcome};
