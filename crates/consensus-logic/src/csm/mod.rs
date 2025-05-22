//! Consensus state machine
//!
//! This is responsible for managing the final view of the checkpointing state,
//! tracking unrecognized state from L1, and determining the basis for which
//! unfinalized blocks are committed.
// TODO clean up this module so that specific items are directly exported and
// modules don't have to be

pub mod chain_tracker;
pub mod client_transition;
pub mod config;
pub mod ctl;
pub mod message;
pub mod worker;
