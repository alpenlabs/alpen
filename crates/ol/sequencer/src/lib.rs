//! Simplified sequencer for OL blocks.
//!
//! This crate provides a clean, worker-less sequencer for the new OL architecture.
//! Block template caching is handled by the block-assembly service.

mod duty;
mod error;
mod extraction;
mod types;

pub use duty::{BlockSigningDuty, CheckpointSigningDuty, Duty, Expiry};
pub use error::Error;
pub use extraction::extract_duties;
pub use strata_ol_block_assembly::BlockasmHandle;
pub use types::{BlockCompletionData, BlockGenerationConfig, BlockTemplate, BlockTemplateExt};
