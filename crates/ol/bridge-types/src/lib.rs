//! Alpen-local bridge types.
//!
//! These are bridge-domain types specific to alpen's OL/CL layer that do not
//! belong to the on-chain ASM bridge protocol. The shared ASM bridge protocol
//! types (`OperatorIdx`, `OperatorSelection`, `OperatorBitmap`,
//! `WithdrawalCommand`, `WithdrawOutput`, etc.) live in `strata-bridge-types`
//! from the asm repo and are imported separately.

mod bridge;
mod bridge_ops;
mod deposit;

// Re-export commonly used types
pub use bridge::PublickeyTable;
pub use bridge_ops::{DepositIntent, WithdrawalBatch, WithdrawalIntent};
pub use deposit::{DepositDescriptor, DepositDescriptorError};
