#![allow(dead_code)] // TODO: remove this once `finalized_tip` fn is used in `ForkChoiceManager`.
//! Consensus validation logic and core state machine

pub mod block_assembly;
pub mod chain_transition;
pub mod client_transition;
pub mod credential;
pub mod ctl;
pub mod duties;
pub mod duty_executor;
pub mod duty_extractor;
pub mod fork_choice_manager;
pub mod genesis;
pub mod l1_handler;
pub mod message;
pub mod reorg;
pub mod state_tracker;
pub mod status;
pub mod sync_manager;
pub mod unfinalized_tracker;
pub mod worker;

pub mod errors;
