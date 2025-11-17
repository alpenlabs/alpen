//! Commit Transaction Parser and Validation
//!
//! This module provides functionality for parsing and validating Bitcoin commit transactions
//! that follow the SPS-50 specification for the Strata bridge protocol.
//!
//! ## Commit Transaction Structure
//!
//! A commit transaction is posted by an operator to commit to a specific deposit and its
//! associated payouts among many available deposits. There is 1 Commit transaction,
//! 1 Uncontested Payout Transaction and 1 Contested Payout Transaction for each Deposit
//! transaction.
//!
//! ### Inputs
//! - **Operator Inputs** (flexible): Any inputs controlled by the operator making the commitment
//!   - The operator is responsible for funding this transaction from their own UTXOs
//!   - No specific input structure is enforced - it's up to the operator to handle funding
//!
//! ### Outputs
//! 1. **OP_RETURN Output (Index 0)** (required): Contains SPS-50 tagged data with:
//!    - Magic number (4 bytes): Protocol instance identifier
//!    - Subprotocol ID (1 byte): Bridge v1 subprotocol identifier
//!    - Transaction type (1 byte): Commit transaction type
//!    - Auxiliary data (4 bytes):
//!      - Deposit index (4 bytes, big-endian u32): Index of the deposit being committed to
//!
//! Additional outputs may be present (e.g., change outputs) but are ignored during validation.
mod parse;

pub use parse::{COMMIT_TX_AUX_DATA_LEN, CommitInfo, parse_commit_tx};
