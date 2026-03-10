//! Simplified sequencer for OL blocks.
//!
//! This crate provides a clean, worker-less sequencer for the new OL architecture.
//! Block template caching is handled by the block-assembly service.

mod builder;
mod duty;
mod error;
mod extraction;
pub(crate) mod input;
pub(crate) mod service;
pub mod signing;
mod types;

pub use builder::SequencerBuilder;
pub use duty::{BlockSigningDuty, CheckpointSigningDuty, Duty, Expiry};
pub use error::Error;
pub use extraction::extract_duties;
pub use service::{SequencerContext, SequencerContextError, SequencerServiceStatus};
pub use signing::{sign_checkpoint, sign_header};
pub use strata_ol_block_assembly::BlockasmHandle;
pub use types::{BlockCompletionData, BlockGenerationConfig, BlockTemplate, BlockTemplateExt};
