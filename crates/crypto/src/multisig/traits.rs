use std::{fmt::Debug, hash::Hash};

use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};

use crate::multisig::errors::MultisigError;

/// A generic trait representing a cryptographic scheme for multisignature operations.
///
/// This trait focuses on the core cryptographic operations: key aggregation and signature
/// verification. Higher-level multisig workflow is handled by generic functions that use this
/// trait.
pub trait CryptoScheme: Clone + Debug + Send + Sync + 'static {
    /// The type used to represent a public key in this scheme.
    type PubKey: Clone
        + Debug
        + PartialEq
        + Eq
        + Hash
        + Send
        + Sync
        + BorshSerialize
        + BorshDeserialize
        + for<'a> Arbitrary<'a>;

    /// The type used to represent a signature in this scheme.
    type Signature: Clone
        + Debug
        + PartialEq
        + Eq
        + Send
        + Sync
        + BorshSerialize
        + BorshDeserialize
        + Default;

    /// The type used to represent an aggregated public key.
    type AggregatedKey: Clone + Debug + PartialEq + Eq + Send + Sync;

    /// Aggregates multiple public keys into a single aggregated key.
    ///
    /// # Arguments
    /// * `keys` - An iterator over public keys to aggregate
    ///
    /// # Returns
    /// Returns the aggregated public key on success, or an error if aggregation fails.
    fn aggregate<'k>(
        keys: impl Iterator<Item = &'k Self::PubKey>,
    ) -> Result<Self::AggregatedKey, MultisigError>
    where
        Self::PubKey: 'k;

    /// Verifies a signature against a message hash using an aggregated public key.
    ///
    /// # Arguments
    /// * `key` - The aggregated public key to verify against
    /// * `message_hash` - The message hash that was signed
    /// * `signature` - The signature to verify
    ///
    /// # Returns
    /// Returns `true` if the signature is valid, `false` otherwise.
    fn verify(
        key: &Self::AggregatedKey,
        message_hash: &[u8; 32],
        signature: &Self::Signature,
    ) -> bool;
}
