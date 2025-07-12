//! Anchor State Machine (ASM) state transition logic for Strata.
//!
//! This crate defines [`asm_stf`], the function that advances the global
//! `AnchorState` by validating a Bitcoin block, routing its transactions to
//! registered subprotocols and finalising their execution.  The surrounding
//! modules provide the handler and stage infrastructure used by the STF.

mod manager;
mod spec;
mod stage;
mod transition;
mod tx_filter;
mod types;

pub use spec::StrataAsmSpec;
pub use transition::{asm_stf, pre_process_asm};
pub use tx_filter::group_txs_by_subprotocol;
pub use types::{AsmStfInput, AsmStfOutput};
