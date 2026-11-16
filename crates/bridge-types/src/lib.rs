//! Bridge types for the Strata protocol.
//!
//! This crate contains types related to bridge operations, including operator management,
//! bridge messages, and bridge state management.

// Re-export bridge types that are canonically defined in ASM.
pub use strata_asm_bridge_types::*;
// The deposit descriptor is canonically defined in strata-common; re-export it at the path OL
// code already uses.
pub use strata_identifiers::{DepositDescriptor, DepositDescriptorError};
