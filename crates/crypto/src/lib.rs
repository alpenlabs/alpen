//! Cryptographic primitives.

pub mod multisig;
pub mod schnorr;
#[cfg(feature = "test-utils")]
pub mod test_utils;

pub use schnorr::*;
