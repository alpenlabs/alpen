//! Bridge V1 transaction assembly utilities.
//!
//! This crate provides functionality for creating and parsing bridge transactions
//! following the SPS-50 specification for the Bridge V1 subprotocol.
//!
//! This crate defines the canonical transaction structures for Bridge V1.
//! Other components must be compatible with the transaction definitions defined here.

pub mod constants;
pub mod deposit;
pub mod deposit_request;
pub mod errors;
pub mod parser;
pub mod slash;
pub mod unstake;
pub mod withdrawal_fulfillment;

pub use constants::BRIDGE_V1_SUBPROTOCOL_ID;

#[cfg(any(test, feature = "test-utils"))]
pub mod test_utils;
