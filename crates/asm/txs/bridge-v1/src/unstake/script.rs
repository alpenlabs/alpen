use bitcoin::{
    ScriptBuf, XOnlyPublicKey,
    opcodes::all::{OP_CHECKSIGVERIFY, OP_EQUAL, OP_EQUALVERIFY, OP_SHA256, OP_SIZE},
    script::Instruction,
};

/// Instruction indices for the stake connector script
const PUBKEY_INDEX: usize = 0;
const STAKE_HASH_INDEX: usize = 6;

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
/// at the expected positions. Full structural validation happens by reconstructing
/// the script and comparing byte-for-byte.
///
/// Returns `None` if the basic structure doesn't allow parameter extraction.
fn extract_script_params(script: &ScriptBuf) -> Option<([u8; 32], [u8; 32])> {
    // Extract pubkey from instruction at PUBKEY_INDEX
    let pubkey = match script.instructions().nth(PUBKEY_INDEX) {
        Some(Ok(Instruction::PushBytes(bytes))) if bytes.len() == 32 => {
            bytes.as_bytes().try_into().ok()?
        }
        _ => return None,
    };

    // Extract stake_hash from instruction at STAKE_HASH_INDEX
    let stake_hash = match script.instructions().nth(STAKE_HASH_INDEX) {
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
    (script == &expected_script).then_some((pubkey, stake_hash_bytes))
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
    fn test_roundtrips_and_raw_extraction() {
        let cases = vec![
            ([0x01u8; 32], [0x42u8; 32]),
            ([0x02u8; 32], [0xAAu8; 32]),
            ([0x03u8; 32], [0x00u8; 32]),
        ];

        for (secret_key_bytes, stake_hash) in cases {
            let pubkey = create_pubkey_from_secret(secret_key_bytes);
            let script = stake_connector_script(stake_hash, pubkey);

            let (extracted_pubkey, extracted_hash) =
                validate_and_extract_script_params(&script).expect("valid script must parse");
            assert_eq!(extracted_pubkey, pubkey);
            assert_eq!(extracted_hash, stake_hash);

            let (raw_pubkey_bytes, raw_hash_bytes) =
                extract_script_params(&script).expect("raw extraction must succeed");
            assert_eq!(raw_pubkey_bytes, pubkey.serialize());
            assert_eq!(raw_hash_bytes, stake_hash);
        }
    }

    #[test]
    fn test_rejects_invalid_scripts() {
        use bitcoin::opcodes::all::{OP_CHECKSIG, OP_DROP, OP_PUSHNUM_1};

        let stake_hash = [0x42u8; 32];
        let pubkey = create_pubkey_from_secret([0x04u8; 32]);
        let pubkey_bytes = pubkey.serialize();

        // Deliberately force a non-minimal push for the stake hash (OP_PUSHDATA1 + len).
        // Canonical encoding here is a direct 0x20 push opcode; OP_PUSHDATA1 should be rejected
        // even though the bytes that follow are still 32 bytes of stake hash.
        let non_minimal_stake_push_script = {
            let mut script_bytes = Vec::new();
            script_bytes.extend_from_slice(&[0x20]);
            script_bytes.extend_from_slice(&pubkey_bytes);
            script_bytes.push(OP_CHECKSIGVERIFY.to_u8());
            script_bytes.push(OP_SIZE.to_u8());
            script_bytes.push(0x01);
            script_bytes.push(0x20);
            script_bytes.push(OP_EQUALVERIFY.to_u8());
            script_bytes.push(OP_SHA256.to_u8());
            script_bytes.push(0x4c); // OP_PUSHDATA1
            script_bytes.push(0x20);
            script_bytes.extend_from_slice(&stake_hash);
            script_bytes.push(OP_EQUAL.to_u8());
            ScriptBuf::from_bytes(script_bytes)
        };

        let invalid_scripts: Vec<(&str, ScriptBuf)> = vec![
            (
                "wrong opcode",
                ScriptBuf::builder()
                    .push_slice(pubkey_bytes)
                    .push_opcode(OP_CHECKSIG)
                    .push_opcode(OP_SIZE)
                    .push_int(0x20)
                    .push_opcode(OP_EQUALVERIFY)
                    .push_opcode(OP_SHA256)
                    .push_slice(stake_hash)
                    .push_opcode(OP_EQUAL)
                    .into_script(),
            ),
            (
                "extra trailing instruction",
                ScriptBuf::builder()
                    .push_slice(pubkey_bytes)
                    .push_opcode(OP_CHECKSIGVERIFY)
                    .push_opcode(OP_SIZE)
                    .push_int(0x20)
                    .push_opcode(OP_EQUALVERIFY)
                    .push_opcode(OP_SHA256)
                    .push_slice(stake_hash)
                    .push_opcode(OP_EQUAL)
                    .push_opcode(OP_DROP)
                    .into_script(),
            ),
            (
                "truncated before hash verification",
                ScriptBuf::builder()
                    .push_slice(pubkey_bytes)
                    .push_opcode(OP_CHECKSIGVERIFY)
                    .push_opcode(OP_SIZE)
                    .push_int(0x20)
                    .into_script(),
            ),
            (
                "stake hash position is not a push",
                ScriptBuf::builder()
                    .push_slice(pubkey_bytes)
                    .push_opcode(OP_CHECKSIGVERIFY)
                    .push_opcode(OP_SIZE)
                    .push_int(0x20)
                    .push_opcode(OP_EQUALVERIFY)
                    .push_opcode(OP_SHA256)
                    .push_opcode(OP_PUSHNUM_1)
                    .push_opcode(OP_EQUAL)
                    .into_script(),
            ),
            (
                "invalid pubkey bytes",
                ScriptBuf::builder()
                    .push_slice([0x00u8; 32])
                    .push_opcode(OP_CHECKSIGVERIFY)
                    .push_opcode(OP_SIZE)
                    .push_int(0x20)
                    .push_opcode(OP_EQUALVERIFY)
                    .push_opcode(OP_SHA256)
                    .push_slice(stake_hash)
                    .push_opcode(OP_EQUAL)
                    .into_script(),
            ),
            (
                "pushes short pubkey",
                ScriptBuf::builder()
                    .push_slice([0x03u8; 31])
                    .push_opcode(OP_CHECKSIGVERIFY)
                    .push_opcode(OP_SIZE)
                    .push_int(0x20)
                    .push_opcode(OP_EQUALVERIFY)
                    .push_opcode(OP_SHA256)
                    .push_slice(stake_hash)
                    .push_opcode(OP_EQUAL)
                    .into_script(),
            ),
            (
                "pushes short stake hash",
                ScriptBuf::builder()
                    .push_slice(pubkey_bytes)
                    .push_opcode(OP_CHECKSIGVERIFY)
                    .push_opcode(OP_SIZE)
                    .push_int(0x20)
                    .push_opcode(OP_EQUALVERIFY)
                    .push_opcode(OP_SHA256)
                    .push_slice([0xAAu8; 31])
                    .push_opcode(OP_EQUAL)
                    .into_script(),
            ),
            ("non-minimal stake hash push", non_minimal_stake_push_script),
        ];

        for (name, script) in invalid_scripts {
            assert!(
                validate_and_extract_script_params(&script).is_none(),
                "{name} must be rejected"
            );
        }
    }
}
