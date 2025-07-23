//! Anchor State Machine (ASM) state transition logic for Strata.
//!
//! This crate defines [`asm_stf`], the function that advances the global
//! `AnchorState` by validating a Bitcoin block, routing its transactions to
//! registered subprotocols and finalising their execution.  The surrounding
//! modules provide the handler and stage infrastructure used by the STF.

mod manager;
mod preprocess;
mod spec;
mod stage;
mod stf;
mod tx_filter;
mod types;

pub use preprocess::pre_process_asm;
pub use spec::StrataAsmSpec;
pub use stf::{asm_stf, asm_stf_with_genesis};
pub use tx_filter::group_txs_by_subprotocol;
pub use types::{AsmStfInput, AsmStfOutput};
