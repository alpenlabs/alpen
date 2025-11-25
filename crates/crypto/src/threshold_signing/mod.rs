//! Threshold signing module for multi-party signature schemes.
//!
//! This module provides two sub-modules:
//! - `musig2`: MuSig2 key aggregation for the bridge subprotocol (N-of-N Schnorr)
//! - `indexed_signatures`: Individual ECDSA signatures for the admin subprotocol (M-of-N threshold)

pub mod indexed_signatures;
pub mod musig2;

// Re-export commonly used types from indexed_signatures
pub use indexed_signatures::{
    verify_threshold_signatures, CompressedPublicKey, IndexedSignature, SignatureSet,
    ThresholdConfig, ThresholdConfigUpdate, ThresholdSigningError,
};
// Re-export MuSig2 key aggregation
pub use musig2::{aggregate_schnorr_keys, Musig2Error};
