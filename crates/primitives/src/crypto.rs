//! Cryptographic primitives - now moved to strata-crypto crate.
//!
//! This module exists only for backward compatibility. New code should
//! use `strata_crypto::schnorr` directly.
//!
//! The schnorr signing/verification logic has been moved to the
//! strata-crypto crate to reduce dependencies in the primitives crate.
//!
//! Note: Due to circular dependency issues (crypto depends on primitives),
//! downstream crates should import from strata-crypto directly:
//! `use strata_crypto::schnorr::{sign_schnorr_sig, verify_schnorr_sig};`
