//! Deposit Transaction Parser and Validation
//!
//! This module provides functionality for parsing and validating Bitcoin deposit transactions
//! that follow the SPS-50 specification for the Strata bridge protocol.
//!
//! ## Deposit Transaction Structure
//!
//! A deposit transaction is obtained by spending a Deposit Request Transaction (DRT) and has
//! the following structure:
//!
//! ### Inputs
//! - **First Input** (required): Spends a P2TR output from a Deposit Request Transaction
//!   - Contains a witness with a Taproot signature from the aggregated operator key
//!   - The signature proves authorization to create the deposit
//!   - Additional inputs may be present but are ignored
//!
//! ### Outputs
//! 1. **OP_RETURN Output (Index 0)** (required): Contains SPS-50 tagged data with:
//!    - Magic number (4 bytes): Protocol instance identifier
//!    - Subprotocol ID (1 byte): Bridge v1 subprotocol identifier
//!    - Transaction type (1 byte): Deposit transaction type
//!    - Auxiliary data (≤74 bytes):
//!      - Deposit index (4 bytes, big-endian u32)
//!      - Tapscript root hash (32 bytes) from the spent DRT
//!      - Destination address (variable length)
//!
//! 2. **P2TR Deposit Output (Index 1)** (required): The actual deposit containing:
//!    - Pay-to-Taproot script with aggregated operator key as internal key
//!    - No merkle root (key-spend only)
//!    - The deposited Bitcoin amount
//!
//! Additional outputs may be present but are ignored during validation.
//!
//! ## Security Model
//!
//! The tapscript root hash from the DRT is critical for maintaining the bridge's security
//! guarantees. It ensures that only properly authorized deposits (with presigned withdrawal
//! transactions) can mint tokens, preserving the 1-of-N trust assumption for withdrawals.
mod parse;
mod validation;

pub const DEPOSIT_OUTPUT_INDEX: u32 = 1;

pub use parse::{DepositInfo, MIN_DEPOSIT_TX_AUX_DATA_LEN, parse_deposit_tx};
pub use validation::{validate_deposit_output_lock, validate_drt_spending_signature};
