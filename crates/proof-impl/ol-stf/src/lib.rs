//! Orchestration Layer State Transition Function (OL STF) proof statements.
//!
//! This crate provides the proof statements for zero-knowledge verification of the
//! Orchestration Layer's state transition function. It defines the logic that runs
//! inside the ZKVM guest to verify that OL blocks are processed correctly according
//! to the protocol rules.
#[cfg(not(target_os = "zkvm"))]
pub mod program;

mod statements;

pub use statements::process_ol_stf;
