//! Anchor State Machine (ASM) state transition logic for Strata.
//!
//! This crate defines [`compute_asm_transition`], the function that advances the global
//! `AnchorState` by validating a Bitcoin block, routing its transactions to
//! registered subprotocols and finalising their execution.  The surrounding
//! modules provide the handler and stage infrastructure used by the STF.

mod manager;
#[cfg(feature = "preprocess")]
mod preprocess;
mod stage;
mod stf;
mod tx_filter;
mod types;

#[cfg(feature = "preprocess")]
pub use preprocess::pre_process_asm;
pub use stf::compute_asm_transition;
pub use tx_filter::group_txs_by_subprotocol;
#[cfg(feature = "preprocess")]
pub use types::AsmPreProcessOutput;
pub use types::{AsmStfInput, AsmStfOutput};
