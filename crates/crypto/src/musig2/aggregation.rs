//! MuSig2 key aggregation for Schnorr signatures.

use musig2::KeyAggContext;
use secp256k1::{Parity, PublicKey, XOnlyPublicKey};
use strata_identifiers::Buf32;
use thiserror::Error;

/// Errors that can occur during MuSig2 operations.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum Musig2Error {
    /// Invalid public key at a specific index.
    #[error("invalid public key at index {index}: {reason}")]
    InvalidPubKey {
        /// The index of the invalid key.
        index: usize,
        /// The reason why the key is invalid.
        reason: String,
    },

    /// Key aggregation context creation failed.
    #[error("key aggregation context creation failed: {reason}")]
    AggregationContextFailed {
        /// The reason why context creation failed.
        reason: String,
    },
}

/// Aggregates a collection of Schnorr public keys using MuSig2 key aggregation.
///
/// This function is used by the bridge subprotocol to create an aggregated
/// public key from all operator keys. The resulting key is used for:
/// - Generating deposit addresses (taproot)
/// - Verifying aggregated signatures on withdrawal transactions
///
/// # Arguments
/// * `keys` - An iterator over 32-byte x-only public keys to aggregate
///
/// # Returns
/// Returns the aggregated x-only public key on success.
///
/// # Errors
/// * `Musig2Error::InvalidPubKey` - If a key is not a valid x-only public key
/// * `Musig2Error::AggregationContextFailed` - If MuSig2 context creation fails
///
/// # Example
/// ```ignore
/// use strata_crypto::threshold_signature::musig2::aggregate_schnorr_keys;
/// use strata_identifiers::Buf32;
///
/// let keys: Vec<Buf32> = operator_keys.iter().map(|k| k.into()).collect();
/// let aggregated_key = aggregate_schnorr_keys(keys.iter())?;
/// ```
pub fn aggregate_schnorr_keys<'k>(
    keys: impl Iterator<Item = &'k Buf32>,
) -> Result<XOnlyPublicKey, Musig2Error> {
    let public_keys = keys
        .enumerate()
        .map(|(index, op)| {
            XOnlyPublicKey::from_slice(op.as_ref())
                .map_err(|e| Musig2Error::InvalidPubKey {
                    index,
                    reason: e.to_string(),
                })
                .map(|x_only| PublicKey::from_x_only_public_key(x_only, Parity::Even))
        })
        .collect::<Result<Vec<_>, Musig2Error>>()?;

    let agg_pubkey = KeyAggContext::new(public_keys)
        .map_err(|e| Musig2Error::AggregationContextFailed {
            reason: e.to_string(),
        })?
        .aggregated_pubkey::<PublicKey>()
        .x_only_public_key()
        .0;

    Ok(agg_pubkey)
}

#[cfg(test)]
mod tests {
    use secp256k1::{Secp256k1, SecretKey};

    use super::*;

    #[test]
    fn test_aggregate_two_keys() {
        let secp = Secp256k1::new();

        let sk1 = SecretKey::from_slice(&[0x01; 32]).unwrap();
        let sk2 = SecretKey::from_slice(&[0x02; 32]).unwrap();

        let pk1 = sk1.x_only_public_key(&secp).0;
        let pk2 = sk2.x_only_public_key(&secp).0;

        let buf1 = Buf32::from(pk1.serialize());
        let buf2 = Buf32::from(pk2.serialize());

        let keys = [buf1, buf2];
        let result = aggregate_schnorr_keys(keys.iter());

        assert!(result.is_ok());
    }

    #[test]
    fn test_aggregate_invalid_key() {
        let invalid_key = Buf32::from([0u8; 32]); // All zeros is invalid
        let keys = [invalid_key];

        let result = aggregate_schnorr_keys(keys.iter());
        assert!(matches!(result, Err(Musig2Error::InvalidPubKey { .. })));
    }
}
