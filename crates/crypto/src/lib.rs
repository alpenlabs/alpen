//! Cryptographic primitives.

// FIXME this stub had to be moved to make a refactor work
pub use strata_primitives::crypto::*;
pub mod groth16_verifier;
pub mod multisig;
#[cfg(feature = "test-utils")]
pub mod test_utils;
