//! EE DA extraction helpers for bounded L1 ranges.

mod fetch;
mod reassemble;
mod scan;

#[cfg(test)]
mod test_utils;

pub use fetch::{
    fetch_range, fetch_range_with_policy, FetchError, FetchPolicy, FetchReader, InvalidBlockRange,
    L1BlockData,
};
pub use reassemble::{reassemble_da_blobs, ReassembleError};
pub use scan::{
    parse_chunked_envelope, peek_commit_marker, scan_block, CommitMarker, ParsedEnvelope, ScanError,
};
