//! ECDSA signature verification for threshold signatures.

use super::{IndexedSignature, SignatureSet, ThresholdConfig, ThresholdSignatureError};

mod ecdsa;

/// Verifies a set of ECDSA signatures against a threshold configuration.
///
/// # Arguments
///
/// * `config` - The threshold configuration containing authorized public keys
/// * `signatures` - Slice of indexed ECDSA signatures to verify
/// * `message_hash` - The 32-byte message hash that was signed
///
/// # Verification Steps
///
/// 1. Construct and validate SignatureSet (checks for duplicates)
/// 2. Check that the number of signatures meets the threshold
/// 3. For each signature, verify that:
///    - The signer index is within bounds
///    - The ECDSA signature is valid for the corresponding public key
///
/// # Returns
///
/// * `Ok(())` if all signatures are valid and threshold is met
/// * `Err(ThresholdSignatureError)` otherwise
pub fn verify_threshold_signatures(
    config: &ThresholdConfig,
    signatures: &[IndexedSignature],
    message_hash: &[u8; 32],
) -> Result<(), ThresholdSignatureError> {
    // Construct and validate SignatureSet (checks for duplicates)
    let signature_set = SignatureSet::new(signatures.to_vec())?;

    // Check threshold is met
    if signature_set.len() < config.threshold() as usize {
        return Err(ThresholdSignatureError::InsufficientSignatures {
            provided: signature_set.len(),
            required: config.threshold() as usize,
        });
    }

    // Delegate to ECDSA-specific verification
    ecdsa::verify_ecdsa_signatures(config, &signature_set, message_hash)
}

#[cfg(test)]
mod tests {
    use secp256k1::{Secp256k1, SecretKey};

    use super::*;
    use crate::threshold_signature::indexed::CompressedPublicKey;

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
        let sig0 = ecdsa::sign_ecdsa_recoverable(&message_hash, &sk1);
        let sig1 = ecdsa::sign_ecdsa_recoverable(&message_hash, &sk2);

        let signatures = vec![
            IndexedSignature::new(0, sig0),
            IndexedSignature::new(1, sig1),
        ];

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
        let sig1 = ecdsa::sign_ecdsa_recoverable(&message_hash, &sk2);

        let signatures = vec![IndexedSignature::new(1, sig1)];

        let result = verify_threshold_signatures(&config, &signatures, &message_hash);
        assert!(matches!(
            result,
            Err(ThresholdSignatureError::InsufficientSignatures { .. })
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
        let sig0 = ecdsa::sign_ecdsa_recoverable(&message_hash, &sk1);
        // Sign with wrong message
        let sig1_wrong = ecdsa::sign_ecdsa_recoverable(&wrong_message_hash, &sk2);

        let signatures = vec![
            IndexedSignature::new(0, sig0),
            IndexedSignature::new(1, sig1_wrong),
        ];

        let result = verify_threshold_signatures(&config, &signatures, &message_hash);
        assert!(matches!(
            result,
            Err(ThresholdSignatureError::InvalidSignature { index: 1 })
        ));
    }

    #[test]
    fn test_verify_wrong_signer() {
        let (sk1, pk1) = generate_keypair(1);
        let (_sk2, pk2) = generate_keypair(2);

        let config = ThresholdConfig::try_new(vec![pk1, pk2], 2).unwrap();

        let message_hash = [0xAB; 32];

        // Both signatures from sk1, but one claims to be from index 1
        let sig0 = ecdsa::sign_ecdsa_recoverable(&message_hash, &sk1);
        let sig1_from_wrong_key = ecdsa::sign_ecdsa_recoverable(&message_hash, &sk1);

        let signatures = vec![
            IndexedSignature::new(0, sig0),
            IndexedSignature::new(1, sig1_from_wrong_key), /* Claims to be key 1, but signed by
                                                            * key 0 */
        ];

        let result = verify_threshold_signatures(&config, &signatures, &message_hash);
        assert!(matches!(
            result,
            Err(ThresholdSignatureError::InvalidSignature { index: 1 })
        ));
    }

    #[test]
    fn test_verify_index_out_of_bounds() {
        let (sk1, pk1) = generate_keypair(1);
        let (sk2, pk2) = generate_keypair(2);

        let config = ThresholdConfig::try_new(vec![pk1, pk2], 2).unwrap();

        let message_hash = [0xAB; 32];

        let sig0 = ecdsa::sign_ecdsa_recoverable(&message_hash, &sk1);
        let sig_oob = ecdsa::sign_ecdsa_recoverable(&message_hash, &sk2);

        let signatures = vec![
            IndexedSignature::new(0, sig0),
            IndexedSignature::new(99, sig_oob), // Out of bounds
        ];

        let result = verify_threshold_signatures(&config, &signatures, &message_hash);
        assert!(matches!(
            result,
            Err(ThresholdSignatureError::SignerIndexOutOfBounds { index: 99, .. })
        ));
    }

    #[test]
    fn test_verify_duplicate_signer_rejected() {
        let (sk1, pk1) = generate_keypair(1);
        let (_sk2, pk2) = generate_keypair(2);

        let config = ThresholdConfig::try_new(vec![pk1, pk2], 2).unwrap();

        let message_hash = [0xAB; 32];

        // Same signer index twice (should fail)
        let sig0 = ecdsa::sign_ecdsa_recoverable(&message_hash, &sk1);
        let sig0_dup = ecdsa::sign_ecdsa_recoverable(&message_hash, &sk1);

        let signatures = vec![
            IndexedSignature::new(0, sig0),
            IndexedSignature::new(0, sig0_dup),
        ];

        let result = verify_threshold_signatures(&config, &signatures, &message_hash);
        assert!(matches!(
            result,
            Err(ThresholdSignatureError::DuplicateSignerIndex(0))
        ));
    }
}
