//! Bridge types for the Strata protocol.
//!
//! This crate contains types related to bridge operations, including operator management,
//! bridge messages, and bridge state management.

mod deposit;

// Re-export bridge types that are canonically defined in ASM.
// Export OL-local bridge types that are not available in ASM.
pub use deposit::{DepositDescriptor, DepositDescriptorError};
pub use strata_asm_bridge_types::*;
