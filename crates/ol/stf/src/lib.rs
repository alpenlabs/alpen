//! Orchestration Layer (OL) State Transition Function.
//!
//! This crate implements the state transition logic for OL blocks, including:
//! - Transaction execution (snark account updates)
//! - Epoch sealing with L1 log processing
//! - Fund transfers via the linear [`Coin`](strata_ledger_types::Coin) abstraction
//! - System account handling (bridge gateway)
//!
//! ## Architecture
//!
//! - `stf`: Core block execution logic
//! - `update`: Generic `send_message`/`send_transfer` handling all account types
//! - `ledger`: Adapts STF to [`LedgerInterface`](strata_ledger_types::LedgerInterface) for
//!   snark-acct-sys
//! - `asm`: L1 log processing during epoch sealing (deposits, checkpoints)
//! - `system_handlers`: Special handling for system accounts like bridge gateway

pub(crate) mod asm;
pub mod context;
pub mod error;
mod exec_output;
mod ledger;
mod stf;
pub(crate) mod system_handlers;
pub(crate) mod update;
mod validation;

pub use exec_output::ExecOutput;
pub use stf::*;
pub use validation::*;
