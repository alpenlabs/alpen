#![allow(dead_code)] // TODO: remove this once `finalized_tip` fn is used in `ForkChoiceManager`.
//! Consensus validation logic and core state machine

pub mod checkpoint_sync;
pub mod checkpoint_verification;
pub mod csm;
pub mod fork_choice_manager;
pub mod genesis;
pub mod sync_manager;
pub mod tip_update;
pub mod unfinalized_tracker;

pub mod errors;
