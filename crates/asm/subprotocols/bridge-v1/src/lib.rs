//! Bridge V1 Subprotocol
//!
//! This crate implements the Strata bridge subprotocol.
//!
//! The bridge manages Bitcoin deposits, operators, withdrawal assignments,
//! between Bitcoin L1 and the orchestration layer.
//!
//! # Architecture
//!
//! The bridge consists of several key components:
//!
//! - **Operators**: Entities that process withdrawals and maintain bridge security
//! - **Deposits**: Bitcoin UTXOs locked to N/N multisig operator addresses
//! - **Assignments**: Task assignments linking deposits to specific operators
//! - **Withdrawals**: Commands for operators to release funds from the multisig.
//!
//! # Usage
//!
//! The main entry point is [`subprotocol::BridgeV1Subproto`] which implements the `Subprotocol`
//! trait for integration with the Anchor State Machine.

mod errors;
mod handler;
mod msgs;
mod parser;
mod state;
mod subprotocol;

pub use errors::*;
pub use msgs::BridgeIncomingMsg;
pub use state::{
    BridgeV1Config, BridgeV1State,
    withdrawal::{WithdrawalCommand, WithdrawalRequest},
};
pub use subprotocol::BridgeV1Subproto;
