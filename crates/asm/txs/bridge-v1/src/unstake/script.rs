use bitcoin::{
    ScriptBuf, XOnlyPublicKey,
    opcodes::all::{OP_CHECKSIGVERIFY, OP_EQUAL, OP_EQUALVERIFY, OP_SHA256, OP_SIZE},
    script::Instruction,
};

/// Builds the stake connector script used in unstaking transactions.
///
/// This script validates:
/// - A signature from the N/N aggregated key
/// - A 32-byte preimage whose SHA256 hash matches the provided stake_hash
///
/// This function serves dual purposes:
/// 1. Building scripts for new transactions
/// 2. Validating parsed scripts via reconstruction and comparison
pub fn stake_connector_script(stake_hash: [u8; 32], pubkey: XOnlyPublicKey) -> ScriptBuf {
    ScriptBuf::builder()
        // Verify the signature
        .push_slice(pubkey.serialize())
        .push_opcode(OP_CHECKSIGVERIFY)
        // Verify size of preimage is 32 bytes
        .push_opcode(OP_SIZE)
        .push_int(0x20)
        .push_opcode(OP_EQUALVERIFY)
        // Verify the preimage matches the hash
        .push_opcode(OP_SHA256)
        .push_slice(stake_hash)
        .push_opcode(OP_EQUAL)
        .into_script()
}

/// Extracts the two dynamic parameters (pubkey and stake_hash) from a stake connector script.
///
/// This is a minimal extraction that only validates the 32-byte push instructions exist
/// at the expected positions (0 and 6). Full structural validation happens by reconstructing
/// the script and comparing byte-for-byte.
///
/// Returns `None` if the basic structure doesn't allow parameter extraction.
fn extract_script_params(script: &ScriptBuf) -> Option<([u8; 32], [u8; 32])> {
    let mut instructions = script.instructions();

    // Extract pubkey from instruction 0 (first push)
    let pubkey = match instructions.next() {
        Some(Ok(Instruction::PushBytes(bytes))) if bytes.len() == 32 => {
            bytes.as_bytes().try_into().ok()?
        }
        _ => return None,
    };

    // Skip instructions 1-5 (static opcodes - will be validated via reconstruction)
    for _ in 0..5 {
        instructions.next();
    }

    // Extract stake_hash from instruction 6
    let stake_hash = match instructions.next() {
        Some(Ok(Instruction::PushBytes(bytes))) if bytes.len() == 32 => {
            bytes.as_bytes().try_into().ok()?
        }
        _ => return None,
    };

    Some((pubkey, stake_hash))
}

/// Validates a stake connector script and extracts its parameters.
///
/// This function performs complete validation by:
/// 1. Extracting the pubkey and stake_hash from the script
/// 2. Reconstructing what the script SHOULD be with those parameters
/// 3. Comparing byte-for-byte with the original script
///
/// Returns the extracted parameters only if the script exactly matches the canonical
/// `stake_connector_script` output. This ensures the script structure is correct.
///
/// # Returns
/// - `Some((pubkey, stake_hash))` if the script is valid and matches the canonical structure
/// - `None` if the script is malformed or doesn't match the expected structure
pub fn validate_and_extract_script_params(
    script: &ScriptBuf,
) -> Option<(XOnlyPublicKey, [u8; 32])> {
    // STEP 1: Extract the two dynamic parameters
    let (pubkey_bytes, stake_hash_bytes) = extract_script_params(script)?;

    // STEP 2: Parse pubkey to ensure it's a valid X-only public key
    let pubkey = XOnlyPublicKey::from_slice(&pubkey_bytes).ok()?;

    // STEP 3: Reconstruct what the script SHOULD be
    let expected_script = stake_connector_script(stake_hash_bytes, pubkey);

    // STEP 4: Byte-for-byte comparison - only return params if script matches exactly
    if script == &expected_script {
        Some((pubkey, stake_hash_bytes))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use bitcoin::secp256k1::{Keypair, Secp256k1, SecretKey};

    use super::*;

    /// Helper function to create a valid XOnlyPublicKey from a secret key
    fn create_pubkey_from_secret(secret_bytes: [u8; 32]) -> XOnlyPublicKey {
        let secp = Secp256k1::new();
        let secret_key = SecretKey::from_slice(&secret_bytes).unwrap();
        let keypair = Keypair::from_secret_key(&secp, &secret_key);
        XOnlyPublicKey::from_keypair(&keypair).0
    }

    #[test]
    fn test_validate_and_extract_valid_script() {
        // Create a valid script with known parameters
        let stake_hash = [0x42u8; 32];
        let pubkey = create_pubkey_from_secret([0x01u8; 32]);

        let script = stake_connector_script(stake_hash, pubkey);

        // Validation should succeed and return the exact parameters
        let result = validate_and_extract_script_params(&script);
        assert!(result.is_some());

        let (extracted_pubkey, extracted_hash) = result.unwrap();
        assert_eq!(extracted_pubkey, pubkey);
        assert_eq!(extracted_hash, stake_hash);
    }

    #[test]
    fn test_validate_and_extract_wrong_opcode() {
        use bitcoin::opcodes::all::OP_CHECKSIG;

        // Create a script with wrong opcode (OP_CHECKSIG instead of OP_CHECKSIGVERIFY)
        let stake_hash = [0x42u8; 32];
        let pubkey = create_pubkey_from_secret([0x02u8; 32]);
        let pubkey_bytes = pubkey.serialize();

        let wrong_script = ScriptBuf::builder()
            .push_slice(pubkey_bytes)
            .push_opcode(OP_CHECKSIG) // Wrong! Should be OP_CHECKSIGVERIFY
            .push_opcode(OP_SIZE)
            .push_int(0x20)
            .push_opcode(OP_EQUALVERIFY)
            .push_opcode(OP_SHA256)
            .push_slice(stake_hash)
            .push_opcode(OP_EQUAL)
            .into_script();

        // Validation should fail
        let result = validate_and_extract_script_params(&wrong_script);
        assert!(result.is_none());
    }

    #[test]
    fn test_validate_and_extract_extra_instructions() {
        use bitcoin::opcodes::all::OP_DROP;

        // Create a script with an extra instruction at the end
        let stake_hash = [0x42u8; 32];
        let pubkey = create_pubkey_from_secret([0x03u8; 32]);
        let pubkey_bytes = pubkey.serialize();

        let wrong_script = ScriptBuf::builder()
            .push_slice(pubkey_bytes)
            .push_opcode(OP_CHECKSIGVERIFY)
            .push_opcode(OP_SIZE)
            .push_int(0x20)
            .push_opcode(OP_EQUALVERIFY)
            .push_opcode(OP_SHA256)
            .push_slice(stake_hash)
            .push_opcode(OP_EQUAL)
            .push_opcode(OP_DROP) // Extra instruction!
            .into_script();

        // Validation should fail
        let result = validate_and_extract_script_params(&wrong_script);
        assert!(result.is_none());
    }

    #[test]
    fn test_validate_and_extract_missing_instructions() {
        // Create a script with missing instructions
        let pubkey = create_pubkey_from_secret([0x04u8; 32]);
        let pubkey_bytes = pubkey.serialize();

        let wrong_script = ScriptBuf::builder()
            .push_slice(pubkey_bytes)
            .push_opcode(OP_CHECKSIGVERIFY)
            .push_opcode(OP_SIZE)
            // Missing the rest of the instructions
            .into_script();

        // Validation should fail during extraction
        let result = validate_and_extract_script_params(&wrong_script);
        assert!(result.is_none());
    }

    #[test]
    fn test_validate_and_extract_wrong_stake_hash() {
        // Build script that uses non-canonical push encoding for the stake_hash
        let original_stake_hash = [0x42u8; 32];
        let pubkey = create_pubkey_from_secret([0x05u8; 32]);
        let pubkey_bytes = pubkey.serialize();

        // Create a script where we use a different push opcode encoding (OP_PUSHDATA1)
        // for the stake_hash instead of the direct push that the builder uses.
        // This will have the correct structure for extraction but won't match
        // the canonical reconstruction which uses minimal push encoding.
        let mut script_bytes = Vec::new();
        script_bytes.extend_from_slice(&[0x20]); // Push 32 bytes for pubkey
        script_bytes.extend_from_slice(&pubkey_bytes);
        script_bytes.push(OP_CHECKSIGVERIFY.to_u8());
        script_bytes.push(OP_SIZE.to_u8());
        script_bytes.push(0x01); // Push 1 byte
        script_bytes.push(0x20); // The value 32
        script_bytes.push(OP_EQUALVERIFY.to_u8());
        script_bytes.push(OP_SHA256.to_u8());
        // Add a malformed push: use OP_PUSHDATA1 instead of direct push
        script_bytes.push(0x4c); // OP_PUSHDATA1
        script_bytes.push(0x20); // 32 bytes
        script_bytes.extend_from_slice(&original_stake_hash);
        script_bytes.push(OP_EQUAL.to_u8());

        let corrupted_script = ScriptBuf::from_bytes(script_bytes);

        // Validation should fail because the reconstruction uses direct push, not OP_PUSHDATA1
        let result = validate_and_extract_script_params(&corrupted_script);
        assert!(result.is_none());
    }

    #[test]
    fn test_validate_and_extract_invalid_pubkey() {
        // Create a script with invalid public key bytes (all zeros is invalid for X-only pubkey)
        let stake_hash = [0x42u8; 32];
        let invalid_pubkey_bytes = [0x00u8; 32]; // Invalid X-only public key

        let script = ScriptBuf::builder()
            .push_slice(invalid_pubkey_bytes)
            .push_opcode(OP_CHECKSIGVERIFY)
            .push_opcode(OP_SIZE)
            .push_int(0x20)
            .push_opcode(OP_EQUALVERIFY)
            .push_opcode(OP_SHA256)
            .push_slice(stake_hash)
            .push_opcode(OP_EQUAL)
            .into_script();

        // Validation should fail when trying to parse the pubkey
        let result = validate_and_extract_script_params(&script);
        assert!(result.is_none());
    }

    #[test]
    fn test_validate_and_extract_wrong_push_sizes() {
        // Create a script with wrong push sizes
        let stake_hash = [0x42u8; 32];

        // Use 31 bytes instead of 32 for pubkey
        let wrong_size_pubkey = [0x03u8; 31];

        let wrong_script = ScriptBuf::builder()
            .push_slice(wrong_size_pubkey) // Wrong size!
            .push_opcode(OP_CHECKSIGVERIFY)
            .push_opcode(OP_SIZE)
            .push_int(0x20)
            .push_opcode(OP_EQUALVERIFY)
            .push_opcode(OP_SHA256)
            .push_slice(stake_hash)
            .push_opcode(OP_EQUAL)
            .into_script();

        // Validation should fail during extraction
        let result = validate_and_extract_script_params(&wrong_script);
        assert!(result.is_none());
    }

    #[test]
    fn test_roundtrip_multiple_parameters() {
        // Test with various different parameters to ensure consistency
        let test_cases = vec![
            ([0x11u8; 32], [0xAAu8; 32]),
            ([0x22u8; 32], [0x00u8; 32]),
            ([0x33u8; 32], [0x84u8; 32]),
        ];

        for (secret_key_bytes, stake_hash) in test_cases {
            let pubkey = create_pubkey_from_secret(secret_key_bytes);
            let script = stake_connector_script(stake_hash, pubkey);

            let result = validate_and_extract_script_params(&script);
            assert!(result.is_some());

            let (extracted_pubkey, extracted_hash) = result.unwrap();
            assert_eq!(extracted_pubkey, pubkey);
            assert_eq!(extracted_hash, stake_hash);
        }
    }
}
