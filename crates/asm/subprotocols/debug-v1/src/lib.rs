//! Debug Subprotocol
//!
//! This crate implements a debug subprotocol for ASM.
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
//! - **`FAKE_ASM_LOG_TX_TYPE`**: Injects arbitrary log messages into the ASM
//! - **`FAKE_WITHDRAW_INTENT_TX_TYPE`**: Creates fake withdrawal intent for the bridge
//!
//! # Security
//!
//! This subprotocol is intended for testing only and should never be enabled
//! in non-testing runtime. it's available when ASM initiated with `DebugAsmSpec`.

// Silence unused dependency warnings for these crates
use borsh as _;
use serde as _;
use strata_asm_logs as _;
use strata_msg_fmt as _;
use strata_primitives as _;
use thiserror as _;

mod constants;
mod subprotocol;
mod txs;

pub use subprotocol::DebugSubproto;
