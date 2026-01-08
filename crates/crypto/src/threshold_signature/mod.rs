//! Threshold signature module for multi-party signature schemes.
//!
//! This module provides two sub-modules:
//! - `musig2`: MuSig2 key aggregation for the bridge subprotocol (N-of-N Schnorr)
//! - `indexed`: Individual ECDSA signatures for the admin subprotocol (M-of-N threshold)

pub mod indexed;

// Re-export commonly used types from indexed
pub use indexed::{
    verify_threshold_signatures, IndexedSignature, SignatureSet, ThresholdConfig,
    ThresholdConfigUpdate, ThresholdSignatureError,
};
