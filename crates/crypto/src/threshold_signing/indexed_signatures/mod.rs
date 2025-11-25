//! Individual ECDSA signature set for threshold signing (M-of-N).
//!
//! This module provides types and functions for verifying a set of individual
//! ECDSA signatures against a threshold configuration. Used by the admin
//! subprotocol for hardware wallet compatibility.

mod config;
mod errors;
mod pubkey;
mod signature;
mod verification;

pub use config::{ThresholdConfig, ThresholdConfigUpdate};
pub use errors::ThresholdSigningError;
pub use pubkey::CompressedPublicKey;
pub use signature::{IndexedSignature, SignatureSet};
pub use verification::verify_threshold_signatures;
