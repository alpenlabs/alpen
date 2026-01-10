//! Bridge types for the Strata protocol.
//!
//! This crate contains types related to bridge operations, including operator management,
//! bridge messages, and bridge state management.

mod bridge;
mod bridge_ops;
mod operator;

// Re-export commonly used types
pub use bridge::PublickeyTable;
pub use bridge_ops::{DepositIntent, WithdrawalBatch, WithdrawalIntent};
pub use operator::OperatorIdx;
