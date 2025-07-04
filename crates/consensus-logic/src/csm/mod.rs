//! Consensus state machine
//!
//! This is responsible for managing the final view of the checkpointing state,
//! tracking unrecognized state from L1, and determining the basis for which
//! unfinalized blocks are committed.
// TODO clean up this module so that specific items are directly exported and
// modules don't have to be

mod chain_tracker;
pub mod client_transition;
mod common;
pub mod config;
pub mod csm_worker;
pub mod ctl;
pub mod message;
mod orphan_tracker;
pub mod worker;
