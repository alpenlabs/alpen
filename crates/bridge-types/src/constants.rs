//! Constants for bridge-related types.

/// The size (in bytes) of a [`musig2::PartialSignature`].
pub(crate) const MUSIG2_PARTIAL_SIG_SIZE: usize = 32;

/// The size (in bytes) of a [`musig2::NonceSeed`].
pub(crate) const NONCE_SEED_SIZE: usize = 32;

/// The size (in bytes) of a [`musig2::PubNonce`].
pub(crate) const PUB_NONCE_SIZE: usize = 66;

/// The size (in bytes) of a [`musig2::SecNonce`].
pub(crate) const SEC_NONCE_SIZE: usize = 64;
