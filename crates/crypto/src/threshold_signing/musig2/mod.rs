//! MuSig2 key aggregation for N-of-N Schnorr signatures.
//!
//! This module provides key aggregation functionality for the bridge subprotocol,
//! where all operators must sign (N-of-N). The aggregated public key is used
//! for taproot addresses and signature verification.

mod aggregation;

pub use aggregation::{aggregate_schnorr_keys, Musig2Error};
