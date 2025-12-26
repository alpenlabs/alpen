//! Deposit Request Transaction (DRT) Building and Parsing
//!
//! This module provides functionality for building and parsing Bitcoin deposit request
//! transactions that follow the SPS-50 specification for the Strata bridge protocol.
//!
//! ## Deposit Request Transaction Structure
//!
//! A deposit request transaction (DRT) is created by users who want to deposit Bitcoin
//! into the Strata bridge. The operators then spend this DRT to create a Deposit Transaction.
//!
//! ### Inputs
//! - Any valid Bitcoin inputs funding the transaction (user's choice)
//! - No specific structure required - flexible to user wallet setup
//!
//! ### Outputs
//! 1. **OP_RETURN Output (Index 0)** (required): Contains SPS-50 tagged data with:
//!    - Magic number (4 bytes): Protocol instance identifier (e.g., `b"alpn"`)
//!    - Subprotocol ID (1 byte): Bridge v1 subprotocol identifier (value: 2)
//!    - Transaction type (1 byte): Deposit request transaction type (value: 0)
//!    - Auxiliary data (>32 bytes):
//!      - Recovery public key (32 bytes, x-only): For takeback script after timeout
//!      - Destination address (variable length): Where sBTC will be minted on the execution layer
//!
//! 2. **P2TR Deposit Request Output (Index 1)** (required): The deposit being locked:
//!    - Pay-to-Taproot script with aggregated N-of-N operator key as internal key
//!    - Taproot merkle root commits to single takeback tapscript: ```text <depositor's xonly public
//!      key> OP_CHECKSIGVERIFY <D> OP_CSV ``` where D is the number of blocks before depositor can
//!      reclaim funds
//!    - Contains `d + dep_fee` sats (deposit amount + mining fee for deposit transaction)
//!
//! 3. **Change Output** (optional): Returns excess funds to user-controlled address
//!
//! ## Security Model
//!
//! The recovery public key in the OP_RETURN data is critical for user fund safety. It allows
//! users to reclaim their Bitcoin if operators fail to process the deposit within the timeout
//! period.

mod aux;
mod info;
mod lock;
mod parse;

pub const DRT_OUTPUT_INDEX: usize = 1;

pub use aux::DrtHeaderAux;
pub use info::DepositRequestInfo;
pub use lock::{build_deposit_request_spend_info, create_deposit_request_locking_script};
pub use parse::parse_drt;
