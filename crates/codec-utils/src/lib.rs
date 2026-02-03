//! Utils around the strata-codec system that don't belong in the upstream crates.

mod rkyv_shim;
mod ssz_shim;

pub use rkyv_shim::*;
pub use ssz_shim::*;
