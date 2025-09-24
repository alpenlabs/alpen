//! Constants for bridge-related types.

use strata_primitives::l1::BitcoinAmount;

/// The size (in bytes) of a [`musig2::PartialSignature`].
pub(crate) const MUSIG2_PARTIAL_SIG_SIZE: usize = 32;

/// The size (in bytes) of a [`musig2::NonceSeed`].
pub(crate) const NONCE_SEED_SIZE: usize = 32;

/// The size (in bytes) of a [`musig2::PubNonce`].
pub(crate) const PUB_NONCE_SIZE: usize = 66;

/// The size (in bytes) of a [`musig2::SecNonce`].
pub(crate) const SEC_NONCE_SIZE: usize = 64;

// TODO make this not hardcoded!
/// The denomination for withdrawal batches in Bitcoin.
pub const WITHDRAWAL_DENOMINATION: BitcoinAmount = BitcoinAmount::from_int_btc(10);
