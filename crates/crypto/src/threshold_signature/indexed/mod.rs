//! ECDSA signature set for threshold signatures (M-of-N).
//!
//! This module provides types and functions for verifying a set of
//! ECDSA signatures against a threshold configuration. Used by the admin
//! subprotocol for hardware wallet compatibility.

mod config;
mod errors;
mod pubkey;
mod signature;
mod verification;

pub use config::{ThresholdConfig, ThresholdConfigUpdate, MAX_SIGNERS};
pub use errors::ThresholdSignatureError;
pub use pubkey::CompressedPublicKey;
pub use signature::{IndexedSignature, SignatureSet};
pub use verification::verify_threshold_signatures;
