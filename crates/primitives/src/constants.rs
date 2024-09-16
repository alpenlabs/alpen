//! Constants for magic numbers and strings used in the primitives.

/// The size (in bytes) of a [`musig2::PartialSignature`].
pub const MUSIG2_PARTIAL_SIG_SIZE: usize = 32;

/// The size (in bytes) of a [`musig2::NonceSeed`].
pub const NONCE_SEED_SIZE: usize = 32;

/// The size (in bytes) of a [`musig2::PubNonce`].
pub const PUB_NONCE_SIZE: usize = 66;
