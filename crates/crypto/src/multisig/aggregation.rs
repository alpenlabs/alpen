use bitvec::slice::BitSlice;

use crate::multisig::{
    config::MultisigConfig,
    errors::MultisigError,
    traits::CryptoScheme,
};

/// Returns the aggregated public key from an iterator of operator public keys.
///
/// This is now a generic function that delegates to the specific crypto scheme.
///
/// # Errors
///
/// Returns an error if key aggregation fails for the given scheme.
pub fn generate_agg_pubkey<'k, S: CryptoScheme>(
    keys: impl Iterator<Item = &'k S::PubKey>,
) -> Result<S::AggregatedKey, MultisigError>
where
    S::PubKey: 'k,
{
    S::aggregate(keys)
}

/// Aggregates public keys selected by the given bit indices.
///
/// This function uses the provided bit slice to select a subset of keys from the
/// multisig configuration and aggregates them into a single public key using
/// the specified cryptographic scheme.
///
/// # Arguments
///
/// * `config` - The multisig configuration containing all keys
/// * `indices` - A bit slice where each set bit (1) indicates a key to include in the aggregation.
///   The bit at index `i` corresponds to `config.keys[i]`.
///
/// # Returns
///
/// Returns the aggregated public key on success, or an error if:
/// - Insufficient keys are selected (fewer than the threshold)
/// - Key aggregation fails
///
/// # Errors
///
/// * `S::Error` - Scheme-specific errors including insufficient keys or aggregation failures
pub fn aggregate<S: CryptoScheme>(
    config: &MultisigConfig<S>,
    indices: &BitSlice,
) -> Result<S::AggregatedKey, MultisigError> {
    S::aggregate_subset(&config.keys, indices, config.threshold as usize)
}
