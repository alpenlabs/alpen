//! Simplified sequencer for OL blocks.
//!
//! This crate provides a clean, worker-less template management system
//! for the new OL architecture. Key differences from the old sequencer:
//! - No worker thread pattern
//! - Templates embedded in duties
//! - TTL-based cache with automatic cleanup
//! - Direct async operations

mod cache;
mod duty;
mod error;
mod extraction;
mod template_manager;
mod types;

pub use duty::{BlockSigningDuty, CheckpointSigningDuty, Duty, Expiry};
pub use error::Error;
pub use extraction::extract_duties;
pub use strata_ol_block_assembly::BlockasmHandle;
pub use template_manager::TemplateManager;
pub use types::{BlockCompletionData, BlockGenerationConfig, BlockTemplate, BlockTemplateExt};
