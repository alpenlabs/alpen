//! Constants for magic numbers and strings used in the primitives.

use std::sync::LazyLock;

use bitcoin::{bip32::ChildNumber, XOnlyPublicKey};
use secp256k1::hashes::{sha256, Hash};

/// The size (in bytes) of a [`musig2::PartialSignature`].
pub const MUSIG2_PARTIAL_SIG_SIZE: usize = 32;

/// The size (in bytes) of a [`musig2::NonceSeed`].
pub const NONCE_SEED_SIZE: usize = 32;

/// The size (in bytes) of a [`musig2::PubNonce`].
pub const PUB_NONCE_SIZE: usize = 66;

/// The size (in bytes) of a [`musig2::SecNonce`].
pub const SEC_NONCE_SIZE: usize = 64;

/// The size (in bytes) of a Hash (such as [`Txid`](bitcoin::Txid)).
pub const HASH_SIZE: usize = 32;

/// The size (in bytes) Execution environment Address.
pub const EE_ADDRESS_LEN: u8 = 20;

/// Number of blocks after bridge in transaction confirmation that the recovery path can be spent.
pub const RECOVER_DELAY: u32 = 1_008;

/// The number of timestamps used for calculating the median in Bitcoin header verification.
/// According to Bitcoin consensus rules, we need to check that a block's timestamp
/// is not lower than the median of the last eleven blocks' timestamps.
pub const TIMESTAMPS_FOR_MEDIAN: usize = 11;

/// Strata base index for keys.
///
/// # Implementation Details
///
/// The base index is set to 20,000 to ensure that it does not conflict with
/// [BIP-43](https://github.com/bitcoin/bips/blob/master/bip-0043.mediawiki)
/// reserved ranges.
pub const STRATA_BASE_IDX: ChildNumber = ChildNumber::Hardened { index: 20_000 };

/// Strata sequencer index for keys.
pub const STRATA_SEQUENCER_IDX: ChildNumber = ChildNumber::Hardened { index: 10 };

/// Strata operator index for keys.
pub const STRATA_OPERATOR_IDX: ChildNumber = ChildNumber::Hardened { index: 20 };

/// Strata message index for the operator message key.
pub const STRATA_OPERATOR_MESSAGE_IDX: ChildNumber = ChildNumber::Hardened { index: 100 };

/// Strata Wallet index for the operator wallet key.
pub const STRATA_OPERATOR_WALLET_IDX: ChildNumber = ChildNumber::Hardened { index: 101 };

/// Strata derivation path for sequencer's key.
///
/// This corresponds to the path: `m/20000'/10'`.
pub const STRATA_SEQUENCER_DERIVATION_PATH: &[ChildNumber] =
    &[STRATA_BASE_IDX, STRATA_SEQUENCER_IDX];

/// Strata base derivation path for operator's message key.
///
/// This corresponds to the path: `m/20000'/20'`.
pub const STRATA_OPERATOR_BASE_DERIVATION_PATH: &[ChildNumber] =
    &[STRATA_BASE_IDX, STRATA_OPERATOR_IDX];

/// Strata derivation path for operator's key.
///
/// This corresponds to the path: `m/20000'/20'/100`.
///
/// # Warning
///
/// The last path should be hardened as in `m/20000'/20'/100'`.
pub const STRATA_OP_MESSAGE_DERIVATION_PATH: &[ChildNumber] = &[
    STRATA_BASE_IDX,
    STRATA_OPERATOR_IDX,
    STRATA_OPERATOR_MESSAGE_IDX,
];
/// Strata derivation path for operator's wallet key.
///
/// This corresponds to the path: `m/20000'/20'/101`.
///
/// # Warning
///
/// The last path should be hardened as in `m/20000'/20'/101'`.
pub const STRATA_OP_WALLET_DERIVATION_PATH: &[ChildNumber] = &[
    STRATA_BASE_IDX,
    STRATA_OPERATOR_IDX,
    STRATA_OPERATOR_WALLET_IDX,
];
/// A verifiably unspendable public key, produced by hashing a fixed string to a curve group
/// generator.
///
/// This is related to the technique used in [BIP-341](https://github.com/bitcoin/bips/blob/master/bip-0341.mediawiki#constructing-and-spending-taproot-outputs).
///
/// Note that this is _not_ necessarily a uniformly-sampled curve point!
///
/// But this is fine; we only need a generator with no efficiently-computable discrete logarithm
/// relation against the standard generator.
pub const UNSPENDABLE_PUBLIC_KEY_INPUT: &[u8] = b"Strata unspendable";
pub static UNSPENDABLE_PUBLIC_KEY: LazyLock<XOnlyPublicKey> = LazyLock::new(|| {
    XOnlyPublicKey::from_slice(sha256::Hash::hash(UNSPENDABLE_PUBLIC_KEY_INPUT).as_byte_array())
        .expect("valid xonly public key")
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unspendable() {
        // Check that construction of the unspendable key succeeds
        let _ = *UNSPENDABLE_PUBLIC_KEY;
    }
}
