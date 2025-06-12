//! Anchor State Machine (ASM) state transition logic for Strata.
//!
//! This crate defines [`asm_stf`], the function that advances the global
//! `AnchorState` by validating a Bitcoin block, routing its transactions to
//! registered subprotocols and finalising their execution.  The surrounding
//! modules provide the handler and stage infrastructure used by the STF.

mod manager;
mod stage;
mod transition;
mod tx_filter;

pub use transition::{StrataAsmSpec, asm_stf};
