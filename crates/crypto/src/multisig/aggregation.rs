use bitcoin::{key::Parity, secp256k1::PublicKey, XOnlyPublicKey};
use musig2::KeyAggContext;
use strata_primitives::buf::Buf32;

use crate::multisig::errors::KeyAggregationError;

/// Returns the aggregated public key from an iterator of operator public keys.
///
/// # Errors
///
/// Returns an error if any key in the iterator is not a valid x-only public key.
pub fn generate_agg_pubkey<'k>(
    keys: impl Iterator<Item = &'k Buf32>,
) -> Result<XOnlyPublicKey, KeyAggregationError> {
    let public_keys = keys
        .enumerate()
        .map(|(index, op)| {
            XOnlyPublicKey::from_slice(op.as_ref())
                .map_err(|source| KeyAggregationError::InvalidXOnlyKey { index, source })
                .map(|x_only| PublicKey::from_x_only_public_key(x_only, Parity::Even))
        })
        .collect::<Result<Vec<_>, KeyAggregationError>>()?;

    let agg_pubkey = KeyAggContext::new(public_keys)?
        .aggregated_pubkey::<PublicKey>()
        .x_only_public_key()
        .0;

    Ok(agg_pubkey)
}
