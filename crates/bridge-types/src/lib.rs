//! Bridge types for the Strata protocol.
//!
//! This crate contains types related to bridge operations, including operator management,
//! bridge messages, and bridge state management.

mod bridge;
mod bridge_ops;
mod constants;
mod operator;

// Re-export commonly used types
#[cfg(not(target_os = "zkvm"))]
pub use bridge::PublickeyTable;
pub use bridge::{
    Musig2PartialSignature, Musig2PubNonce, Musig2SecNonce, OperatorPartialSig, TxSigningData,
};
pub use bridge_ops::{DepositIntent, WithdrawalBatch, WithdrawalIntent};
pub use operator::{OperatorIdx, OperatorKeyProvider, OperatorPubkeys, StubOpKeyProv};
