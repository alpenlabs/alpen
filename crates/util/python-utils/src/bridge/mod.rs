//! Bridge transaction utilities for Alpen rollup
//! This module provides functionality for creating and signing bridge transactions:
//! - DRT (Deposit Request Transaction)
//!
//! All transactions support MuSig2 multi-signature operations for operator keys.
pub(crate) mod drt;
pub(crate) mod musig_signer;
pub(crate) mod types;

// pub use musig_signer::MusigSigner; // Commented out - only used internally
pub(crate) use drt::*;
