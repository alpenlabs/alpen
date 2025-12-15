//! Checkpoint Subprotocol
//!
//! This crate implements the checkpoint subprotocol for verifying OL STF checkpoints.
//!
//! # Responsibilities
//!
//! - Signature verification using the sequencer credential
//! - State transition validation (epoch, L1/L2 height progression)
//! - ZK proof verification using the checkpoint predicate
//! - Forwarding withdrawal intents to the bridge subprotocol
//! - Processing configuration updates from the admin subprotocol

mod error;
mod handler;
mod state;
mod subprotocol;
mod utils;

pub use state::CheckpointConfig;
pub use subprotocol::CheckpointSubprotocol;
