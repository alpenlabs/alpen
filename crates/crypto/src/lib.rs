//! Cryptographic primitives.

pub mod musig2;
pub mod schnorr;
#[cfg(feature = "test-utils")]
pub mod test_utils;
pub mod threshold_signature;

// Re-export MuSig2 key aggregation
pub use musig2::{aggregate_schnorr_keys, Musig2Error};
pub use schnorr::*;
