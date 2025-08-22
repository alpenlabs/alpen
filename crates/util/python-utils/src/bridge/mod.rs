//! Bridge transaction utilities for Alpen rollup
//! This module provides functionality for creating and signing bridge transactions:
//! - DRT (Deposit Request Transaction)
//!
//! All transactions support MuSig2 multi-signature operations for operator keys.
pub(crate) mod drt;

// Allow(dead_code) is present because the original PR is too big and it's divided into three
// pieces. Other piece use this
#[allow(dead_code)]
pub(crate) mod musig_signer;
#[allow(dead_code)]
pub(crate) mod types;

// pub use musig_signer::MusigSigner; // Commented out - only used internally
pub(crate) use drt::*;
