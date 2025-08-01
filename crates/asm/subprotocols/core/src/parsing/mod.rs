//! This module handles checkpoint data extraction.

mod checkpoint;

// Re-export main parsing functions for convenience
pub(crate) use checkpoint::extract_signed_checkpoint;
