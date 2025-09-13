pub mod config;
pub mod errors;
pub mod schemes;
pub mod signature;
pub mod traits;

// Re-export the default Schnorr scheme
pub use schemes::{aggregate_schnorr_keys, SchnorrScheme};
// Type aliases for Schnorr-based multisig (backward compatibility)
pub type SchnorrMultisigConfig = config::MultisigConfig<SchnorrScheme>;
pub type SchnorrMultisigConfigUpdate = config::MultisigConfigUpdate<SchnorrScheme>;
pub type SchnorrMultisigSignature = signature::MultisigSignature<SchnorrScheme>;

// Re-export the single error type
pub use errors::MultisigError;

use crate::multisig::traits::CryptoScheme;

/// Generic multisig verification function that orchestrates the verification workflow.
///
/// This function takes a multisig configuration, multisig signature, and message hash,
/// then performs the full verification process using the provided cryptographic scheme.
///
/// # Arguments
/// * `config` - The multisig configuration containing keys and threshold
/// * `signature` - The multisig signature containing signer indices and aggregated signature
/// * `message_hash` - The message hash that was signed
///
/// # Returns
/// Returns `Ok(())` if verification succeeds, or an error if:
/// - Signer indices exceed the available keys count
/// - Insufficient keys are selected (fewer than threshold)
/// - Key aggregation fails
/// - Signature verification fails
pub fn verify_multisig<S: CryptoScheme>(
    config: &config::MultisigConfig<S>,
    signature: &signature::MultisigSignature<S>,
    message_hash: &[u8; 32],
) -> Result<(), MultisigError> {
    // Validate that signer indices don't exceed available keys
    let total_indices_count = signature.signer_indices().len();
    if total_indices_count > config.keys().len() {
        return Err(MultisigError::BitVecTooLong {
            bitvec_len: total_indices_count,
            member_count: config.keys().len(),
        });
    }

    // Check threshold
    let selected_count = signature.signer_indices().count_ones();
    if selected_count < config.threshold() as usize {
        return Err(MultisigError::InsufficientKeys {
            provided: selected_count,
            required: config.threshold() as usize,
        });
    }

    // Aggregate selected keys
    let selected_keys = signature
        .signer_indices()
        .iter_ones()
        .map(|index| &config.keys()[index]);
    let aggregated_key = S::aggregate(selected_keys)?;

    // Verify signature
    if !S::verify(&aggregated_key, message_hash, signature.signature()) {
        return Err(MultisigError::InvalidSignature);
    }

    Ok(())
}
