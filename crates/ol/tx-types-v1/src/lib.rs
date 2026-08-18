//! Orchestration layer transaction structures.

mod proofs;
mod transaction;

#[cfg(any(test, feature = "test-utils"))]
pub mod test_utils;

pub use strata_identifiers::{OLTxId, Slot};

/// SSZ-generated types for serialization and merkleization.
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

// Re-export generated SSZ types with their canonical names
pub use ssz_generated::ssz::proofs::*;
pub use ssz_generated::ssz::transaction::*;
