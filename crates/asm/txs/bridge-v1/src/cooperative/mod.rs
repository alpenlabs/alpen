//! Cooperative Withdrawal Transaction Parser and Validation
//!
//! This module provides functionality for parsing and validating Bitcoin cooperative path
//! transactions that follow the SPS-50 specification for the Strata bridge protocol.
//!
//! ## Cooperative Withdrawal Transaction Structure
//!
//! A cooperative transaction is a **multisig transaction** where all active operators pay out
//! the user withdrawal request by jointly spending the requested amount into the user's provided
//! withdrawal address. This transaction has the following structure:
//!
//! ### Inputs
//! - **Bridge Input**: deposit UTXO being spent. Must match the assigned deposit UTXO.
//!
//! ### Outputs
//! 1. **OP_RETURN Output (Index 0)** (required): Contains SPS-50 tagged data with:
//!    - Magic number (4 bytes): Protocol instance identifier
//!    - Subprotocol ID (1 byte): Bridge v1 subprotocol identifier
//!    - Transaction type (1 byte): Cooperative transaction type
//!    - Auxiliary data (≤74 bytes):
//!      - Deposit index (4 bytes, big-endian u32): Index of the original deposit being withdrawn
//!
//! 2. **Withdrawal Output (Index 1)** (required): The actual withdrawal containing:
//!    - The recipient's Bitcoin address (script_pubkey)
//!    - The withdrawal amount (may be less than deposit due to fees)
//!
//! Additional outputs may be present (e.g., change outputs) but are ignored during validation.

mod parse;

pub const BRIDGE_INPUT_INDEX: usize = 0;
pub const USER_WITHDRAWAL_OUTPUT_INDEX: usize = 1;

pub use parse::{COOPERATIVE_TX_AUX_DATA_LEN, CooperativeInfo, parse_cooperative_tx};
