//! Cryptographic primitives.

pub mod groth16_verifier;
pub mod multisig;
pub mod schnorr;
#[cfg(feature = "test-utils")]
pub mod test_utils;
pub mod verifying_key;

pub use schnorr::verify_schnorr_sig;
pub use verifying_key::RollupVerifyingKey;
