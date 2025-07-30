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
//! The main entry point is [`subprotocol::BridgeV1Subproto`] which implements the `Subprotocol`
//! trait for integration with the Anchor State Machine.

mod constants;
mod errors;
mod msgs;
mod state;
mod subprotocol;
mod txs;

pub use constants::BRIDGE_V1_SUBPROTOCOL_ID;
pub use subprotocol::BridgeV1Subproto;
