//! Debug Subprotocol
//!
//! This crate implements a debug subprotocol for the Strata Anchor State Machine (ASM).
//! It provides testing capabilities by allowing injection of mock data through special
//! L1 transactions.
//!
//! # Purpose
//!
//! The debug subprotocol enables testing of ASM components in isolation:
//! - Test the Bridge subprotocol without running the full Orchestration Layer
//! - Test the Orchestration Layer without running the full bridge infrastructure
//! - Inject arbitrary log messages for testing log processing
//!
//! # Transaction Types
//!
//! The debug subprotocol supports the following transaction types:
//!
//! - **`olmsg`**: Injects arbitrary log messages into the ASM
//! - **`fakewithdraw`**: Creates fake withdrawal commands for the bridge
//! - **`unlockdeposit`**: Emits deposit unlock authorization signals
//!
//! # Security
//!
//! This subprotocol is intended for testing only and should never be enabled
//! in production builds. It's protected by feature flags to prevent accidental
//! inclusion in production binaries.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]

// Silence unused dependency warnings for these crates
use serde as _;
use strata_asm_logs as _;
#[cfg(test)]
use strata_test_utils as _;

mod constants;
mod subprotocol;
mod txs;

pub use subprotocol::DebugSubproto;
pub use txs::{FakeWithdrawInfo, OlMsgInfo, UnlockDepositInfo};
