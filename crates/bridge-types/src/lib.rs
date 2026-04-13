//! Bridge types for the Strata protocol.
//!
//! This crate contains types related to bridge operations, including operator management,
//! bridge messages, and bridge state management.

mod bridge;
mod bridge_ops;
mod deposit;

// Re-export bridge types that are canonically defined in ASM.
pub use strata_bridge_types::*;

// Export OL-local bridge types that are not available in ASM.
pub use bridge::PublickeyTable;
pub use bridge_ops::{DepositIntent, WithdrawalBatch, WithdrawalIntent};
pub use deposit::{DepositDescriptor, DepositDescriptorError};
