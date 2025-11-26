//! Commit Transaction Parser and Validation
//!
//! This module provides functionality for parsing and validating Bitcoin commit transactions
//! that follow the SPS-50 specification for the Strata bridge protocol.
//!
//! ## Commit Transaction Structure
//!
//! A commit transaction is posted by an operator to commit to a specific deposit UTXO and its
//! and its corresponding payout transactions (among the available linked deposit UTXOs).
//!
//! ### Inputs
//! - **First Input** (required): Must spend the first output of a Claim transaction
//!   - The input must be locked to the N/N aggregated operator key (key-spend only P2TR)
//!   - While we don't verify it came from a specific Claim transaction during parsing, later
//!     validation must check that it was properly spent from the N/N multisig
//!
//! ### Outputs
//! 1. **OP_RETURN Output (Index 0)** (required): Contains SPS-50 tagged data with:
//!    - Magic number (4 bytes): Protocol instance identifier
//!    - Subprotocol ID (1 byte): Bridge v1 subprotocol identifier
//!    - Transaction type (1 byte): Commit transaction type
//!    - Auxiliary data (8 bytes, encoded using [`strata_codec::Codec`] which uses big-endian for
//!      integers):
//!      - Deposit index (4 bytes, u32): Index of the deposit being committed to
//!      - Game index (4 bytes, u32): Index of the game being committed to
//!
//! 2. **N/N Output (Index 1)** (required): Must be locked to the N/N aggregated operator key
//!    - Pay-to-Taproot script with aggregated operator key as internal key
//!    - No merkle root (key-spend only)
//!    - This output will be spent by the payout transaction
//!
//! Additional outputs may be present (e.g., change outputs) but are ignored during validation.
mod aux;
mod parse;

pub use parse::{COMMIT_TX_AUX_DATA_LEN, CommitInfo, parse_commit_tx};
