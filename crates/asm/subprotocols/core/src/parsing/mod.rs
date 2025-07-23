//! Transaction parsing for the Core subprotocol
//!
//! This module handles parsing of transaction data, including Bitcoin inscriptions
//! and checkpoint data extraction.

pub(crate) mod checkpoint;
pub(crate) mod inscription;

// Re-export main parsing functions for convenience
pub(crate) use checkpoint::extract_signed_checkpoint;
