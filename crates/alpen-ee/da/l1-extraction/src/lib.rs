//! EE DA extraction helpers for bounded L1 ranges.

mod fetch;
mod reassemble;
mod scan;

#[cfg(test)]
mod test_utils;

pub use fetch::{
    fetch_range, fetch_range_with_policy, FetchError, FetchPolicy, FetchReader, InvalidBlockRange,
    L1BlockData, MAX_EXTRACTION_BLOCK_RANGE,
};
pub use reassemble::{reassemble_da_blobs, ReassembleError};
pub use scan::{
    parse_chunked_envelope, peek_commit_marker, scan_blocks, CommitMarker, L1RangeScanner,
    ParsedEnvelope, ScanError,
};
