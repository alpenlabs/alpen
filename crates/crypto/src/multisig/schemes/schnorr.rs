use bitcoin::{key::Parity, secp256k1::PublicKey, XOnlyPublicKey};
use musig2::KeyAggContext;
use strata_primitives::{
    buf::{Buf32, Buf64},
    crypto::verify_schnorr_sig,
};

use crate::multisig::{errors::MultisigError, traits::CryptoScheme};

/// Schnorr signature scheme using MuSig2 key aggregation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SchnorrScheme;

impl CryptoScheme for SchnorrScheme {
    type PubKey = Buf32; // FIXME:?
    type Signature = Buf64;
    type AggregatedKey = Buf32;

    /// Aggregates public keys using MuSig2 key aggregation.
    fn aggregate<'k>(
        keys: impl Iterator<Item = &'k Self::PubKey>,
    ) -> Result<Self::AggregatedKey, MultisigError>
    where
        Self::PubKey: 'k,
    {
        let a = aggregate_schnorr_keys(keys)?;
        Ok(Buf32::from(a.serialize()))
    }

    /// Verifies a Schnorr signature against a message hash using a(n aggregated) public key.
    fn verify(
        key: &Self::AggregatedKey,
        message_hash: &[u8; 32],
        signature: &Self::Signature,
    ) -> bool {
        // Use the existing verification function from strata_primitives
        verify_schnorr_sig(signature, &Buf32::from(*message_hash), key)
    }
}

/// Aggregates a collection of Schnorr public keys using MuSig2 key aggregation.
///
/// # Arguments
/// * `keys` - An iterator over 32-byte public keys to aggregate
///
/// # Returns
/// Returns the aggregated public key on success, or an error if:
/// - Any key is not a valid x-only public key
/// - MuSig2 key aggregation context creation fails
///
/// # Errors
/// * `MultisigError::InvalidPubKey` - If a key is not a valid x-only public key
/// * `MultisigError::AggregationContextFailed` - If MuSig2 context creation fails
pub fn aggregate_schnorr_keys<'k>(
    keys: impl Iterator<Item = &'k Buf32>,
) -> Result<XOnlyPublicKey, MultisigError>
where
{
    let public_keys = keys
        .enumerate()
        .map(|(index, op)| {
            XOnlyPublicKey::from_slice(op.as_ref())
                .map_err(|e| MultisigError::InvalidPubKey {
                    index,
                    reason: e.to_string(),
                })
                .map(|x_only| PublicKey::from_x_only_public_key(x_only, Parity::Even))
        })
        .collect::<Result<Vec<_>, MultisigError>>()?;

    let agg_pubkey = KeyAggContext::new(public_keys)
        .map_err(|e| MultisigError::AggregationContextFailed {
            reason: e.to_string(),
        })?
        .aggregated_pubkey::<PublicKey>()
        .x_only_public_key()
        .0;

    Ok(agg_pubkey)
}
