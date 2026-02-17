//! Debug Subprotocol
//!
//! This crate implements a debug subprotocol for ASM.
//! It provides testing capabilities by allowing injection of mock data through special
//! L1 transactions.
//!
//! # Purpose
//!
//! The debug subprotocol enables testing of ASM components in isolation:
//!
//! - Test the Bridge subprotocol without running the full Orchestration Layer
//! - Test the Orchestration Layer without running the full bridge infrastructure
//! - Inject arbitrary log messages for testing log processing
//!
//! # Transaction Types
//!
//! The debug subprotocol supports the following transaction types:
//!
//! - **`MOCK_ASM_LOG_TX_TYPE` (1)**: Injects arbitrary log messages into the ASM log output,
//!   simulating logs that would normally originate from the bridge subprotocol. Example: Deposit
//!   events (locking funds in n/n multisig)
//!
//! - **`MOCK_WITHDRAW_INTENT_TX_TYPE` (2)**: Creates withdrawal intents that are sent to the bridge
//!   subprotocol, simulating withdrawals from the Orchestration Layer. Format: `[amount: 8 bytes][1
//!   byte B][B bytes: operator index (big-endian)][descriptor: variable]` where B=0 means no
//!   operator selection, B=1..4 encodes the selected operator index, and the descriptor is
//!   self-describing Bitcoin-BOSD format. These messages normally originate from the Checkpoint
//!   subprotocol through inter-protocol messaging.
//!
//! # Subprotocol ID
//!
//! The debug subprotocol uses ID 255 (u8::MAX) to avoid conflicts with production
//! subprotocols, which are assigned incremental IDs starting from 0 as specified in the
//! spec documents.
//!
//! # Security
//!
//! This subprotocol is intended for testing only and should never be enabled
//! in non-testing runtime. It's available when ASM is initiated with `DebugAsmSpec`
//! and should not be included in non-testing runtime builds.

// Silence unused dependency warnings for these crates
use borsh as _;
use serde as _;
use strata_asm_logs as _;
use strata_primitives as _;
use thiserror as _;

mod constants;
mod subprotocol;
mod txs;

pub use subprotocol::DebugSubproto;
