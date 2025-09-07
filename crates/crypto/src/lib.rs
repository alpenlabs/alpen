//! Cryptographic primitives.

// FIXME this stub had to be moved to make a refactor work
pub use strata_primitives::crypto::*;
pub mod groth16_verifier;
pub mod multisig;

// Used by multisig/schemes/schnorr.rs through bitcoin crate
#[allow(unused_extern_crates)]
extern crate secp256k1;
