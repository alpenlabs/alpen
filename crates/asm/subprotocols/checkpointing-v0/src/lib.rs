//! Checkpointing v0 Subprotocol
//!
//! This crate implements the checkpointing v0 subprotocol that maintains feature parity
//! with the current checkpointing system while incorporating SPS-62 concepts where
//! beneficial.
//!
//! # Overview
//!
//! The checkpointing v0 subprotocol is responsible for:
//!
//! - **Checkpoint Verification**: Validates checkpoints using TN1 verification logic
//! - **SPS-50 Envelope Parsing**: Processes envelope transactions
//! - **Upgradability**: Receives upgrade messages from the Administration subprotocol and processes
//!   those inter-protocol messages
//! - **Feature Parity**: Maintains compatibility with existing checkpoint behavior
//! - **Bridge Integration**: Extracts and forwards withdrawal messages to bridge subprotocol
//!
//! # Key Design Decisions
//!
//! - **Current Format Compatibility**: Uses existing checkpoint data structures for verification
//! - **Proof Verification**: Delegates to current groth16 verification until predicates are defined
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
mod error;
mod msgs;
mod subprotocol;
mod types;
mod verification;

// Public re-exports
pub use error::{CheckpointV0Error, CheckpointV0Result};
pub use msgs::CheckpointingIncomingMsg;
pub use subprotocol::{CheckpointingV0Params, CheckpointingV0Subproto};
// Re-export verification functions for testing and integration
pub use verification::process_checkpoint_v0;
