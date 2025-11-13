//! Bridge transaction utilities for Alpen rollup
//!
//! This module provides functionality for creating and signing bridge transactions
//!
//! Transactions
//! - DT (Deposit Transaction)
//! - Withdrawal Fulfillment transactions
//!
//! All transactions support MuSig2 multi-signature operations for operator keys.

pub(crate) mod deposit_request;
pub(crate) mod dt;
pub(crate) mod musig_signer;
pub(crate) mod types;
pub(crate) mod withdrawal;
