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
mod state;
mod subprotocol;
mod validation;

#[cfg(test)]
mod test_utils;

#[allow(
    clippy::all,
    unreachable_pub,
    clippy::allow_attributes,
    clippy::absolute_paths,
    reason = "generated code"
)]
mod ssz_generated {
    include!(concat!(env!("OUT_DIR"), "/generated.rs"));
}

pub use errors::*;
pub use ssz_generated::ssz::state::{
    AssignmentEntry, AssignmentEntryRef, AssignmentTable, AssignmentTableRef, BitmapBytes,
    BridgeV1State, BridgeV1StateRef, DepositEntry, DepositEntryRef, DepositsTable,
    DepositsTableRef, OperatorBitmap, OperatorBitmapRef, OperatorClaimUnlock,
    OperatorClaimUnlockRef, OperatorEntry, OperatorEntryRef, OperatorTable, OperatorTableRef,
    ScriptBytes, WithdrawalCommand, WithdrawalCommandRef,
};
pub use strata_asm_bridge_msgs::{BridgeIncomingMsg, WithdrawOutput};
pub use subprotocol::BridgeV1Subproto;
