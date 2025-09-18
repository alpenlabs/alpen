//! Strata Checkpointing v0 Subprotocol
//!
//! This crate implements the checkpointing v0 subprotocol that maintains feature parity
//! with the current checkpointing system while incorporating SPS-62 concepts where
//! beneficial.
//!
//! # Overview
//!
//! The checkpointing v0 subprotocol is responsible for:
//!
//! - **Checkpoint Verification**: Validates checkpoints using current verification system
//! - **SPS-50 Envelope Parsing**: Processes envelope transactions
//! - **Feature Parity**: Maintains compatibility with existing checkpoint behavior
//! - **Bridge Integration**: Extracts and forwards withdrawal messages to bridge subprotocol
//!
//! # Key Design Decisions
//!
//! - **Current Format Compatibility**: Uses existing checkpoint data structures for verification
//! - **Envelope Parsing**: Follows administration subprotocol pattern for SPS-50 envelope parsing
//! - **Proof Verification Bridge**: Delegates to current groth16 verification until predicates are
//!   defined
//! - **Simplified Auxiliary Input**: Uses basic L1 context instead of full SPS-62 oracles(?)
//!
//! # SPS-62 Compatibility Notes
//!
//! This is checkpointing v0, which prioritizes feature parity with the current system.
//! Future versions will be fully SPS-62 compliant. Current SPS-62 concepts incorporated:
//!
//! - Envelope transaction structure (SPS-50)
//! - Basic verification flow concepts
//! - Placeholder structures for future SPS-62 migration
// Module declarations
mod constants;
mod error;
mod parsing;
mod subprotocol;
mod types;
mod verification;

// Public re-exports
pub use constants::*;
pub use error::{CheckpointV0Error, CheckpointV0Result};
// Re-export parsing functions
pub use parsing::extract_signed_checkpoint_from_envelope;
pub use subprotocol::{CheckpointingV0Config, CheckpointingV0Subproto};
pub use types::*;
// Re-export verification functions for testing and integration
pub use verification::{extract_withdrawal_messages, process_checkpoint_v0};
