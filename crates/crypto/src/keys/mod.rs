//! Cryptographic key types and utilities for Strata.
//!
//! This module provides specialized key types used throughout the Strata codebase:
//!
//! - [`constants`] - Derivation paths and constants for key generation
//! - [`compressed`] - Compressed ECDSA public keys with serialization support
//! - [`even`] - Even parity keys for BIP340 Schnorr signatures and taproot
//! - [`zeroizable`] - Zeroizable wrappers for secure key material handling

pub mod compressed;
pub mod constants;
pub mod even;
pub mod zeroizable;
