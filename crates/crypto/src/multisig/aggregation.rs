use bitcoin::{key::Parity, secp256k1::PublicKey, XOnlyPublicKey};
use bitvec::slice::BitSlice;
use musig2::KeyAggContext;
use strata_primitives::buf::Buf32;

use crate::multisig::{config::MultisigConfig, errors::KeyAggregationError, PubKey};

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

/// Aggregates public keys selected by the given bit indices.
///
/// This function uses the provided bit slice to select a subset of keys from the
/// multisig configuration and aggregates them into a single public key using
/// MuSig2 key aggregation.
///
/// # Arguments
///
/// * `indices` - A bit slice where each set bit (1) indicates a key to include in the aggregation.
///   The bit at index `i` corresponds to `self.keys[i]`.
///
/// # Returns
///
/// Returns the aggregated public key on success, or an error if:
/// - Insufficient keys are selected (fewer than the threshold)
/// - Key aggregation fails
///
/// # Errors
///
/// * `MultisigConfigError::InsufficientKeys` - If fewer keys are selected than required by the
///   threshold
/// * `MultisigConfigError::KeyAggregationFailed` - If the underlying MuSig2 key aggregation process
///   fails
pub fn aggregate(
    config: &MultisigConfig,
    indices: &BitSlice,
) -> Result<PubKey, KeyAggregationError> {
    let selected_count = indices.count_ones();

    let threhold = config.threshold as usize;
    if selected_count < threhold {
        return Err(KeyAggregationError::InsufficientKeys {
            provided: selected_count,
            required: threhold,
        });
    }

    let selected_keys = indices.iter_ones().map(|index| &config.keys[index]);
    let agg_key = generate_agg_pubkey(selected_keys)?.into();

    Ok(agg_key)
}
