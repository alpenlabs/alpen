//! Bridge V1 Subprotocol
//!
//! This crate implements the Strata-Bridge subprotocol.
//!
//! The bridge manages Bitcoin deposits, operator assignments,
//! and withdrawal processing between Bitcoin L1 and the orchestration layer.
//!
//! # Architecture
//!
//! The bridge consists of several key components:
//!
//! - **Operators**: Entities that process withdrawals and maintain bridge security
//! - **Deposits**: Bitcoin UTXOs locked to N/N multisig operator addresses
//! - **Assignments**: Task assignments linking deposits to specific operators
//! - **Withdrawals**: Commands for operators to release funds back to Bitcoin
//!
//! # Usage
//!
//! The main entry point is [`BridgeV1Subproto`] which implements the [`Subprotocol`]
//! trait for integration with the Anchor State Machine.

pub mod constants;
pub mod errors;
pub mod msgs;
pub mod state;
pub mod subprotocol;
pub mod txs;

// Suppress unused dependency warning - this may be used by other parts of the codebase
#[cfg(test)]
use strata_test_utils as _;
