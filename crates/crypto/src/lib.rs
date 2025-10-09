//! Cryptographic primitives.

#![expect(
    unused_crate_dependencies,
    reason = "I think clippy is wrong and I don't want to break dep hierarchy figuring it out"
)]

// FIXME this stub had to be moved to make a refactor work
pub mod groth16_verifier;
pub mod multisig;
pub mod proof_vk;
pub mod schnorr;

#[rustfmt::skip]
#[cfg(feature = "test-utils")]
pub mod test_utils;
