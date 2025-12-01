//! ECDSA-specific signature verification implementation.

use secp256k1::{
    ecdsa::{RecoverableSignature, RecoveryId},
    Message, SECP256K1,
};

use crate::threshold_signature::indexed::{SignatureSet, ThresholdConfig, ThresholdSignatureError};

/// Verifies each ECDSA signature in the set against the corresponding public key.
///
/// This function performs the actual ECDSA signature recovery and verification.
/// It assumes the SignatureSet has already been validated for duplicates.
pub(super) fn verify_ecdsa_signatures(
    config: &ThresholdConfig,
    signatures: &SignatureSet,
    message_hash: &[u8; 32],
) -> Result<(), ThresholdSignatureError> {
    // Create the message for verification
    let message = Message::from_digest_slice(message_hash)
        .map_err(|_| ThresholdSignatureError::InvalidMessageHash)?;

    // Verify each signature
    for indexed_sig in signatures.signatures() {
        // Check index is in bounds
        let index = indexed_sig.index as usize;
        if index >= config.keys().len() {
            return Err(ThresholdSignatureError::SignerIndexOutOfBounds {
                index: indexed_sig.index,
                max: config.keys().len(),
            });
        }

        // Get the expected public key
        let expected_pubkey = config.keys()[index].as_inner();

        // Parse the recoverable signature
        let recovery_id = RecoveryId::from_i32(indexed_sig.recovery_id() as i32)
            .map_err(|_| ThresholdSignatureError::InvalidSignatureFormat)?;

        let recoverable_sig =
            RecoverableSignature::from_compact(&indexed_sig.compact(), recovery_id)
                .map_err(|_| ThresholdSignatureError::InvalidSignatureFormat)?;

        // Recover the public key from the signature
        let recovered_pubkey = SECP256K1
            .recover_ecdsa(&message, &recoverable_sig)
            .map_err(|_| ThresholdSignatureError::InvalidSignature {
                index: indexed_sig.index,
            })?;

        // Verify the recovered key matches the expected key
        if &recovered_pubkey != expected_pubkey {
            return Err(ThresholdSignatureError::InvalidSignature {
                index: indexed_sig.index,
            });
        }
    }

    Ok(())
}

/// Sign a message hash with ECDSA and return a recoverable signature.
///
/// This is a helper function for testing and creating signatures.
#[cfg(test)]
pub(super) fn sign_ecdsa_recoverable(
    message_hash: &[u8; 32],
    secret_key: &secp256k1::SecretKey,
) -> [u8; 65] {
    let message = Message::from_digest_slice(message_hash).expect("32 bytes");
    let sig = SECP256K1.sign_ecdsa_recoverable(&message, secret_key);
    let (recovery_id, compact) = sig.serialize_compact();

    let mut result = [0u8; 65];
    result[0] = recovery_id.to_i32() as u8;
    result[1..65].copy_from_slice(&compact);
    result
}
