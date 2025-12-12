//! Checkpoint Subprotocol
//!
//! This module implements the checkpoint subprotocol, providing
//! verification of OL STF checkpoints including:
//!
//! - Signature verification using the sequencer credential
//! - State transition validation (epoch, slot, L1 height progression)
//! - Extraction of OL DA & OL Logs
//! - Construction of required public parameters for checkpoint zk proof verification
//! - L1→L2 message range verification (accessed via Auxiliary Data)
//! - ZK proof verification using the checkpoint predicate
//! - Forwarding of withdrawal intents to the bridge subprotocol
//! - Processing sequencer pk and checkpoint predicate updates via inter-protocol messages from the
//!   admin subprotocol
//!
//! The checkpoint subprotocol processes checkpoint transactions (SPS50 tagged tx) from the
//! L1, verifies them using auxiliary data and proof verification,
//! then updates the checkpoint subprotocol state and emits appropriate logs.

mod error;
mod handler;
mod msg_handler;
mod state;
mod subprotocol;
mod verification;

pub use state::{CheckpointConfig, CheckpointState};
pub use subprotocol::CheckpointSubprotocol;
