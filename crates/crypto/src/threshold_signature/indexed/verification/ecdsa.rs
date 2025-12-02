//! ECDSA-specific signature verification implementation.

use secp256k1::{
    ecdsa::{RecoverableSignature, RecoveryId},
    Message, SECP256K1,
};

use crate::threshold_signature::indexed::{SignatureSet, ThresholdConfig, ThresholdSignatureError};

/// Normalizes a recovery ID header byte to the raw recovery ID (0-3).
///
/// Hardware wallets (Ledger/Trezor) follow BIP-137 and encode additional address type
/// information in the header byte. This function extracts just the recovery ID needed
/// for ECDSA public key recovery.
///
/// # Header byte formats (BIP-137):
/// - `0-3`: Raw recovery ID (already normalized)
/// - `27-30`: Uncompressed P2PKH (subtract 27)
/// - `31-34`: Compressed P2PKH (subtract 31)
/// - `35-38`: SegWit P2SH-P2WPKH (subtract 35)
/// - `39-42`: Native SegWit P2WPKH (subtract 39)
///
/// # Returns
/// The raw recovery ID (0-3) or an error if the header is invalid.
fn normalize_recovery_id(header: u8) -> Result<i32, ThresholdSignatureError> {
    let recid = match header {
        0..=3 => header,        // Raw format
        27..=30 => header - 27, // Uncompressed P2PKH
        31..=34 => header - 31, // Compressed P2PKH
        35..=38 => header - 35, // SegWit P2SH
        39..=42 => header - 39, // Native SegWit
        _ => return Err(ThresholdSignatureError::InvalidSignatureFormat),
    };
    Ok(recid as i32)
}

/// Verifies each ECDSA signature in the set against the corresponding public key.
///
/// This function performs the actual ECDSA signature recovery and verification.
/// It assumes the SignatureSet has already been validated for duplicates.
///
/// # Hardware Wallet Compatibility
/// Supports signatures from hardware wallets (Ledger/Trezor) that use BIP-137 format
/// with header bytes 27-42, as well as raw format with recovery ID 0-3.
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
        let index = indexed_sig.index() as usize;
        if index > config.keys().len() {
            return Err(ThresholdSignatureError::SignerIndexOutOfBounds {
                index: indexed_sig.index(),
                max: config.keys().len(),
            });
        }

        // Get the expected public key
        let expected_pubkey = config.keys()[index].as_inner();

        // Normalize the recovery ID from BIP-137 header format to raw 0-3
        let recid_raw = normalize_recovery_id(indexed_sig.recovery_id())?;
        let recovery_id = RecoveryId::from_i32(recid_raw)
            .map_err(|_| ThresholdSignatureError::InvalidSignatureFormat)?;

        let recoverable_sig =
            RecoverableSignature::from_compact(&indexed_sig.compact(), recovery_id)
                .map_err(|_| ThresholdSignatureError::InvalidSignatureFormat)?;

        // Recover the public key from the signature
        let recovered_pubkey = SECP256K1
            .recover_ecdsa(&message, &recoverable_sig)
            .map_err(|_| ThresholdSignatureError::InvalidSignature {
                index: indexed_sig.index(),
            })?;

        // Verify the recovered key matches the expected key
        if &recovered_pubkey != expected_pubkey {
            return Err(ThresholdSignatureError::InvalidSignature {
                index: indexed_sig.index(),
            });
        }
    }

    Ok(())
}

/// Sign a message hash with ECDSA and return a recoverable signature.
///
/// This is a helper function for testing and creating signatures.
/// Returns raw format (recovery_id 0-3 in first byte).
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

/// Sign a message hash with ECDSA and return a BIP-137 format signature.
///
/// This simulates hardware wallet output with compressed P2PKH format (header 31-34).
#[cfg(test)]
pub(super) fn sign_ecdsa_bip137(
    message_hash: &[u8; 32],
    secret_key: &secp256k1::SecretKey,
) -> [u8; 65] {
    let mut sig = sign_ecdsa_recoverable(message_hash, secret_key);
    // Convert raw recid (0-3) to BIP-137 compressed P2PKH format (31-34)
    sig[0] += 31;
    sig
}

#[cfg(test)]
mod normalization_tests {
    use super::*;

    #[test]
    fn test_normalize_raw_recovery_id() {
        assert_eq!(normalize_recovery_id(0).unwrap(), 0);
        assert_eq!(normalize_recovery_id(1).unwrap(), 1);
        assert_eq!(normalize_recovery_id(2).unwrap(), 2);
        assert_eq!(normalize_recovery_id(3).unwrap(), 3);
    }

    #[test]
    fn test_normalize_bip137_uncompressed_p2pkh() {
        // 27-30 = uncompressed P2PKH
        assert_eq!(normalize_recovery_id(27).unwrap(), 0);
        assert_eq!(normalize_recovery_id(28).unwrap(), 1);
        assert_eq!(normalize_recovery_id(29).unwrap(), 2);
        assert_eq!(normalize_recovery_id(30).unwrap(), 3);
    }

    #[test]
    fn test_normalize_bip137_compressed_p2pkh() {
        // 31-34 = compressed P2PKH (most common for Ledger/Trezor)
        assert_eq!(normalize_recovery_id(31).unwrap(), 0);
        assert_eq!(normalize_recovery_id(32).unwrap(), 1);
        assert_eq!(normalize_recovery_id(33).unwrap(), 2);
        assert_eq!(normalize_recovery_id(34).unwrap(), 3);
    }

    #[test]
    fn test_normalize_bip137_segwit_p2sh() {
        // 35-38 = SegWit P2SH-P2WPKH
        assert_eq!(normalize_recovery_id(35).unwrap(), 0);
        assert_eq!(normalize_recovery_id(36).unwrap(), 1);
        assert_eq!(normalize_recovery_id(37).unwrap(), 2);
        assert_eq!(normalize_recovery_id(38).unwrap(), 3);
    }

    #[test]
    fn test_normalize_bip137_native_segwit() {
        // 39-42 = Native SegWit P2WPKH
        assert_eq!(normalize_recovery_id(39).unwrap(), 0);
        assert_eq!(normalize_recovery_id(40).unwrap(), 1);
        assert_eq!(normalize_recovery_id(41).unwrap(), 2);
        assert_eq!(normalize_recovery_id(42).unwrap(), 3);
    }

    #[test]
    fn test_normalize_invalid_values() {
        // Values between 4 and 26 are invalid
        for v in 4..27 {
            assert!(normalize_recovery_id(v).is_err());
        }
        // Values above 42 are invalid
        for v in 43..=255 {
            assert!(normalize_recovery_id(v).is_err());
        }
    }
}
