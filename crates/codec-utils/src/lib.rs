//! Utils around the strata-codec system that don't belong in the upstream crates.

mod borsh_shim;
mod ssz_shim;

pub use borsh_shim::*;
pub use ssz_shim::*;
