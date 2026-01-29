//! Checkpoint verification subprotocol for ASM.
//!
//! Processes checkpoint transactions posted to Bitcoin by verifying sequencer signatures
//! and zero-knowledge proofs of correct Orchestration Layer (OL) state transitions. Each
//! checkpoint advances the verified tip, which tracks the last proven OL state, and forwards
//! withdrawal intents to the bridge subprotocol.
//!
//! ## State Management
//!
//! The subprotocol maintains:
//! - Sequencer predicate for signature verification (updatable via admin)
//! - Checkpoint predicate for ZK proof verification (updatable via admin)
//! - Verified tip tracking the last successfully verified checkpoint (epoch, L1 height, L2
//!   commitment)

pub mod errors;
pub mod handler;
pub mod state;
pub mod subprotocol;
pub mod verification;
