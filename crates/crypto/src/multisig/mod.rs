pub mod config;
pub mod errors;
pub mod schemes;
pub mod traits;
pub mod vote;

use bitvec::slice::BitSlice;
// Re-export the default Schnorr scheme
pub use schemes::{aggregate_schnorr_keys, SchnorrScheme};
use strata_primitives::buf::{Buf32, Buf64};
// Re-export the trait for easy access
pub use traits::CryptoScheme;

// Legacy type aliases for backward compatibility
pub type PubKey = Buf32;
pub type Signature = Buf64;

// Type aliases for Schnorr-based multisig (backward compatibility)
pub type MultisigConfig = config::MultisigConfig<SchnorrScheme>;
pub type MultisigConfigUpdate = config::MultisigConfigUpdate<SchnorrScheme>;
pub type AggregatedVote = vote::AggregatedVote<SchnorrScheme>;

// Re-export the single error type
pub use errors::MultisigError;

/// Generic multisig verification function that orchestrates the verification workflow.
///
/// This function takes a multisig configuration, voter indices, message hash, and signature,
/// then performs the full verification process using the provided cryptographic scheme.
///
/// # Arguments
/// * `config` - The multisig configuration containing keys and threshold
/// * `voter_indices` - Bit slice indicating which keys participated in signing
/// * `message_hash` - The message hash that was signed
/// * `signature` - The aggregated signature to verify
///
/// # Returns
/// Returns `Ok(())` if verification succeeds, or an error if:
/// - Insufficient keys are selected (fewer than threshold)
/// - Key aggregation fails
/// - Signature verification fails
pub fn verify_multisig<S: CryptoScheme>(
    config: &config::MultisigConfig<S>,
    voter_indices: &BitSlice,
    message_hash: &[u8; 32],
    signature: &S::Signature,
) -> Result<(), MultisigError> {
    // Check threshold
    let selected_count = voter_indices.count_ones();
    if selected_count < config.threshold() as usize {
        return Err(MultisigError::InsufficientKeys {
            provided: selected_count,
            required: config.threshold() as usize,
        });
    }

    // Aggregate selected keys
    let selected_keys = voter_indices.iter_ones().map(|index| &config.keys()[index]);
    let aggregated_key = S::aggregate(selected_keys)?;

    // Verify signature
    if !S::verify(&aggregated_key, message_hash, signature) {
        return Err(MultisigError::InvalidSignature);
    }

    Ok(())
}
