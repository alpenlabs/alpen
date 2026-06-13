//! Block range witness extraction for EVM proof generation.
//!
//! This crate provides utilities for extracting witness data from arbitrary block ranges.
//! It is a **range-agnostic utility** - the caller determines block range boundaries
//! (e.g., via chunking algorithms, batch boundaries, etc.). This crate simply extracts
//! the witness data needed to prove execution of the given range.

mod cache_db_provider;
mod capture_witness;
mod range_witness_extractor;

pub use cache_db_provider::{AccessedState, CacheDBProvider, StorageKey};
pub use capture_witness::{capture_block_witness, CapturedBlockWitness};
pub use range_witness_extractor::{RangeWitnessData, RangeWitnessExtractor};
