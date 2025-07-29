//! Withdrawal Transaction Parser and Validation
//!
//! This module provides functionality for parsing and validating Bitcoin withdrawal transactions
//! that follow the SPS-50 specification for the Strata bridge protocol.
//!
//! ## Withdrawal Transaction Structure
//!
//! A withdrawal transaction is a **frontpayment transaction** where an operator pays out
//! the withdrawal request before being able to withdraw the corresponding locked deposit.
//! This transaction has the following structure:
//!
//! ### Inputs
//! - **Operator Inputs** (flexible): Any inputs controlled by the operator making the frontpayment
//!   - The operator is responsible for funding this transaction from their own UTXOs
//!   - No specific input structure is enforced - it's up to the operator to handle funding
//!   - The operator will later be able to withdraw the corresponding N/N locked deposit
//!
//! ### Outputs
//! 1. **OP_RETURN Output (Index 0)** (required): Contains SPS-50 tagged data with:
//!    - Magic number (4 bytes): Protocol instance identifier
//!    - Subprotocol ID (1 byte): Bridge v1 subprotocol identifier
//!    - Transaction type (1 byte): Withdrawal transaction type
//!    - Auxiliary data (≤74 bytes):
//!      - Operator index (4 bytes, big-endian u32): Index of the operator processing the withdrawal
//!      - Deposit index (4 bytes, big-endian u32): Index of the original deposit being withdrawn
//!      - Deposit transaction ID (32 bytes): TXID of the deposit transaction being spent
//!
//! 2. **Withdrawal Fulfillment Output (Index 1)** (required): The actual withdrawal containing:
//!    - The recipient's Bitcoin address (script_pubkey)
//!    - The withdrawal amount (may be less than deposit due to fees)
//!
//! Additional outputs may be present (e.g., change outputs) but are ignored during validation.
#[cfg(test)]
pub mod create;
pub mod parse;

pub const USER_WITHDRAWAL_FULFILLMENT_OUTPUT_INDEX: usize = 1;
