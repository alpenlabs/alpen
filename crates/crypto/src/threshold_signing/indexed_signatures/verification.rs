//! ECDSA signature verification for threshold signing.

use secp256k1::{ecdsa::{RecoverableSignature, RecoveryId}, Message, SECP256K1};

use super::{SignatureSet, ThresholdConfig, ThresholdSigningError};

/// Verifies a set of ECDSA signatures against a threshold configuration.
///
/// # Arguments
///
/// * `config` - The threshold configuration containing authorized public keys
/// * `signatures` - The set of indexed ECDSA signatures to verify
/// * `message_hash` - The 32-byte message hash that was signed
///
/// # Verification Steps
///
/// 1. Check that the number of signatures meets the threshold
/// 2. For each signature, verify that:
///    - The signer index is within bounds
///    - The ECDSA signature is valid for the corresponding public key
///
/// # Returns
///
/// * `Ok(())` if all signatures are valid and threshold is met
/// * `Err(ThresholdSigningError)` otherwise
pub fn verify_threshold_signatures(
    config: &ThresholdConfig,
    signatures: &SignatureSet,
    message_hash: &[u8; 32],
) -> Result<(), ThresholdSigningError> {
    // Check threshold is met
    if signatures.len() < config.threshold() as usize {
        return Err(ThresholdSigningError::InsufficientSignatures {
            provided: signatures.len(),
            required: config.threshold() as usize,
        });
    }

    // Create the message for verification
    let message = Message::from_digest_slice(message_hash)
        .map_err(|_| ThresholdSigningError::InvalidMessageHash)?;

    // Verify each signature
    for indexed_sig in signatures.signatures() {
        // Check index is in bounds
        let index = indexed_sig.index as usize;
        if index >= config.keys().len() {
            return Err(ThresholdSigningError::SignerIndexOutOfBounds {
                index: indexed_sig.index,
                max: config.keys().len(),
            });
        }

        // Get the expected public key
        let expected_pubkey = config.keys()[index].as_inner();

        // Parse the recoverable signature
        let recovery_id = RecoveryId::from_i32(indexed_sig.recovery_id() as i32)
            .map_err(|_| ThresholdSigningError::InvalidSignatureFormat)?;

        let recoverable_sig = RecoverableSignature::from_compact(&indexed_sig.compact(), recovery_id)
            .map_err(|_| ThresholdSigningError::InvalidSignatureFormat)?;

        // Recover the public key from the signature
        let recovered_pubkey = SECP256K1
            .recover_ecdsa(&message, &recoverable_sig)
            .map_err(|_| ThresholdSigningError::InvalidSignature {
                index: indexed_sig.index,
            })?;

        // Verify the recovered key matches the expected key
        if &recovered_pubkey != expected_pubkey {
            return Err(ThresholdSigningError::InvalidSignature {
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
fn sign_ecdsa_recoverable(
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

#[cfg(test)]
mod tests {
    use secp256k1::{Secp256k1, SecretKey};

    use super::*;
    use crate::threshold_signing::indexed_signatures::{CompressedPublicKey, IndexedSignature};

    fn generate_keypair(seed: u8) -> (SecretKey, CompressedPublicKey) {
        let secp = Secp256k1::new();
        let mut sk_bytes = [0u8; 32];
        sk_bytes[31] = seed.max(1);
        let sk = SecretKey::from_slice(&sk_bytes).unwrap();
        let pk = CompressedPublicKey::from(secp256k1::PublicKey::from_secret_key(&secp, &sk));
        (sk, pk)
    }

    #[test]
    fn test_verify_threshold_signatures_success() {
        let (sk1, pk1) = generate_keypair(1);
        let (sk2, pk2) = generate_keypair(2);
        let (_sk3, pk3) = generate_keypair(3);

        let config = ThresholdConfig::try_new(vec![pk1, pk2, pk3], 2).unwrap();

        let message_hash = [0xAB; 32];

        // Sign with keys 0 and 1
        let sig0 = sign_ecdsa_recoverable(&message_hash, &sk1);
        let sig1 = sign_ecdsa_recoverable(&message_hash, &sk2);

        let signatures = SignatureSet::new(vec![
            IndexedSignature::new(0, sig0),
            IndexedSignature::new(1, sig1),
        ])
        .unwrap();

        let result = verify_threshold_signatures(&config, &signatures, &message_hash);
        assert!(result.is_ok());
    }

    #[test]
    fn test_verify_insufficient_signatures() {
        let (_sk1, pk1) = generate_keypair(1);
        let (sk2, pk2) = generate_keypair(2);
        let (_sk3, pk3) = generate_keypair(3);

        let config = ThresholdConfig::try_new(vec![pk1, pk2, pk3], 2).unwrap();

        let message_hash = [0xAB; 32];

        // Only sign with one key
        let sig1 = sign_ecdsa_recoverable(&message_hash, &sk2);

        let signatures = SignatureSet::new(vec![IndexedSignature::new(1, sig1)]).unwrap();

        let result = verify_threshold_signatures(&config, &signatures, &message_hash);
        assert!(matches!(
            result,
            Err(ThresholdSigningError::InsufficientSignatures { .. })
        ));
    }

    #[test]
    fn test_verify_invalid_signature() {
        let (sk1, pk1) = generate_keypair(1);
        let (sk2, pk2) = generate_keypair(2);

        let config = ThresholdConfig::try_new(vec![pk1, pk2], 2).unwrap();

        let message_hash = [0xAB; 32];
        let wrong_message_hash = [0xCD; 32];

        // Sign with correct message
        let sig0 = sign_ecdsa_recoverable(&message_hash, &sk1);
        // Sign with wrong message
        let sig1_wrong = sign_ecdsa_recoverable(&wrong_message_hash, &sk2);

        let signatures = SignatureSet::new(vec![
            IndexedSignature::new(0, sig0),
            IndexedSignature::new(1, sig1_wrong),
        ])
        .unwrap();

        let result = verify_threshold_signatures(&config, &signatures, &message_hash);
        assert!(matches!(
            result,
            Err(ThresholdSigningError::InvalidSignature { index: 1 })
        ));
    }

    #[test]
    fn test_verify_wrong_signer() {
        let (sk1, pk1) = generate_keypair(1);
        let (_sk2, pk2) = generate_keypair(2);

        let config = ThresholdConfig::try_new(vec![pk1, pk2], 2).unwrap();

        let message_hash = [0xAB; 32];

        // Both signatures from sk1, but one claims to be from index 1
        let sig0 = sign_ecdsa_recoverable(&message_hash, &sk1);
        let sig1_from_wrong_key = sign_ecdsa_recoverable(&message_hash, &sk1);

        let signatures = SignatureSet::new(vec![
            IndexedSignature::new(0, sig0),
            IndexedSignature::new(1, sig1_from_wrong_key), // Claims to be key 1, but signed by key 0
        ])
        .unwrap();

        let result = verify_threshold_signatures(&config, &signatures, &message_hash);
        assert!(matches!(
            result,
            Err(ThresholdSigningError::InvalidSignature { index: 1 })
        ));
    }

    #[test]
    fn test_verify_index_out_of_bounds() {
        let (sk1, pk1) = generate_keypair(1);
        let (sk2, pk2) = generate_keypair(2);

        let config = ThresholdConfig::try_new(vec![pk1, pk2], 2).unwrap();

        let message_hash = [0xAB; 32];

        let sig0 = sign_ecdsa_recoverable(&message_hash, &sk1);
        let sig_oob = sign_ecdsa_recoverable(&message_hash, &sk2);

        let signatures = SignatureSet::new(vec![
            IndexedSignature::new(0, sig0),
            IndexedSignature::new(99, sig_oob), // Out of bounds
        ])
        .unwrap();

        let result = verify_threshold_signatures(&config, &signatures, &message_hash);
        assert!(matches!(
            result,
            Err(ThresholdSigningError::SignerIndexOutOfBounds { index: 99, .. })
        ));
    }
}
