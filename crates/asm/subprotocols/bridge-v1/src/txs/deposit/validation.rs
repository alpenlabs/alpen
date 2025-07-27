use bitcoin::{
    Amount, ScriptBuf, TapNodeHash, Transaction, TxOut, XOnlyPublicKey,
    hashes::Hash,
    key::TapTweak,
    sighash::{Prevouts, SighashCache},
    taproot::{self},
};
use secp256k1::Message;
use strata_primitives::{buf::Buf32, l1::XOnlyPk};

use crate::{errors::DepositError, txs::deposit::DEPOSIT_OUTPUT_INDEX};

/// Validates that the DRT spending signature in the deposit transaction is valid.
///
/// This function performs Taproot signature validation to verify that the deposit transaction
/// properly spends the Deposit Request Transaction (DRT) with a valid signature from the
/// aggregated operator key.
///
/// The validation process includes:
///
/// 1. **Witness Extraction** - Extracts the signature from the transaction witness
/// 2. **Signature Parsing** - Parses the Taproot signature (supports both 64-byte and 65-byte
///    formats)
/// 3. **Key Derivation** - Derives the tweaked public key from the internal key and merkle root
/// 4. **Sighash Computation** - Computes the transaction sighash for signature verification
/// 5. **Schnorr Verification** - Verifies the Schnorr signature against the tweaked key
///
/// # Parameters
///
/// - `tx` - The deposit transaction that spends the DRT
/// - `drt_tapnode_hash` - The tapscript root hash from the DRT being spent, used to reconstruct the
///   correct taproot address for signature verification
/// - `operators_pubkey` - The aggregated operator public key that should have signed the
///   transaction
/// - `deposit_amount` - The amount from the DRT output being spent
///
/// # Returns
///
/// - `Ok(())` - If the signature is cryptographically valid for the given public key
/// - `Err(DepositError::InvalidSignature)` - If signature validation fails with details about the
///   failure
///
/// # Implementation Details
///
/// Currently uses manual signature verification due to limitations in the bitcoin
/// crate's consensus validation. Future versions should migrate to using the
/// built-in `tx.verify()` method once bitcoinconsensus supports Taproot.
pub fn validate_drt_spending_signature(
    tx: &Transaction,
    drt_tapnode_hash: Buf32,
    operators_pubkey: &XOnlyPk,
    deposit_amount: Amount,
) -> Result<(), DepositError> {
    // Initialize necessary variables and dependencies
    let secp = secp256k1::SECP256K1;

    // FIXME: Use latest version of `bitcoin` once released. The underlying
    // `bitcoinconsensus==0.106` will have support for taproot validation. So here, we just need
    // to create TxOut from operator pubkeys and tapnode hash and call `tx.verify()`.

    // Extract and validate input signature
    let input = tx.input[0].clone();

    // Check if witness is present.
    if input.witness.is_empty() {
        return Err(DepositError::InvalidSignature {
            reason: "No witness data found in transaction input".to_string(),
        });
    }
    let sig_witness = &input.witness[0];

    // rust-bitcoin taproot::Signature handles both both 64-byte (SIGHASH_DEFAULT)
    // and 65-byte (explicit sighash) signatures.
    let taproot_sig = taproot::Signature::from_slice(sig_witness).map_err(|e| {
        DepositError::InvalidSignature {
            reason: format!("Failed to parse taproot signature: {e}"),
        }
    })?;
    let schnorr_sig = taproot_sig.signature;
    let sighash_type = taproot_sig.sighash_type;

    // Parse the internal pubkey and merkle root
    let merkle_root: TapNodeHash = TapNodeHash::from_byte_array(drt_tapnode_hash.0);

    let internal_pubkey = XOnlyPublicKey::from_slice(operators_pubkey.inner().as_bytes()).unwrap();
    let (tweaked_key, _) = internal_pubkey.tap_tweak(secp, Some(merkle_root));

    // Build the scriptPubKey for the UTXO
    let script_pubkey = ScriptBuf::new_p2tr(secp, internal_pubkey, Some(merkle_root));

    let utxos = [TxOut {
        value: deposit_amount,
        script_pubkey,
    }];

    // Compute the sighash
    let prevout = Prevouts::All(&utxos);
    let sighash = SighashCache::new(tx)
        // NOTE: preserving the original sighash_type.
        .taproot_key_spend_signature_hash(0, &prevout, sighash_type)
        .unwrap();

    // Prepare the message for signature verification
    let msg = Message::from_digest(*sighash.as_byte_array());

    // Verify the Schnorr signature
    secp.verify_schnorr(&schnorr_sig, &msg, &tweaked_key.to_x_only_public_key())
        .map_err(|e| DepositError::InvalidSignature {
            reason: format!("Schnorr signature verification failed: {e}"),
        })?;

    Ok(())
}

/// Validates that the deposit output is locked to the N/N aggregated operator key.
///
/// This function verifies that the deposit output at `DEPOSIT_OUTPUT_INDEX` is a P2TR
/// output locked to the provided aggregated operator public key with no merkle root
/// (key-spend only). This ensures the deposited funds can only be spent by the N/N
/// operator set.
///
/// # Parameters
///
/// - `tx` - The deposit transaction to validate
/// - `operators_agg_pubkey` - The aggregated operator public key that should control the deposit
///
/// # Returns
///
/// - `Ok(())` - If the deposit output is properly locked to the operator key
/// - `Err(DepositError)` - If the output is missing, has wrong script type, or wrong key
pub fn validate_deposit_output_lock(
    tx: &Transaction,
    operators_agg_pubkey: &XOnlyPk,
) -> Result<(), DepositError> {
    // Get the deposit output
    let deposit_output = tx
        .output
        .get(DEPOSIT_OUTPUT_INDEX as usize)
        .ok_or(DepositError::MissingOutput(DEPOSIT_OUTPUT_INDEX))?;

    // Extract the internal key from the P2TR script
    let secp = secp256k1::SECP256K1;
    let operators_pubkey = XOnlyPublicKey::from_slice(operators_agg_pubkey.inner().as_bytes())
        .map_err(|_| DepositError::InvalidSignature {
            reason: "Invalid operator public key".to_string(),
        })?;

    // Create expected P2TR script with no merkle root (key-spend only)
    let expected_script = ScriptBuf::new_p2tr(secp, operators_pubkey, None);

    // Verify the deposit output script matches the expected P2TR script
    if deposit_output.script_pubkey != expected_script {
        return Err(DepositError::InvalidSignature {
            reason: "Deposit output is not locked to the aggregated operator key".to_string(),
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use bitcoin::{
        Amount,
        secp256k1::{Secp256k1, SecretKey},
    };
    use strata_primitives::{buf::Buf32, l1::XOnlyPk};

    use super::*;
    use crate::txs::deposit::create::create_test_deposit_tx;

    // Test data constants
    const TEST_DEPOSIT_IDX: u32 = 123;
    const TEST_TAPSCRIPT_ROOT: [u8; 32] = [0xAB; 32];
    const TEST_DESTINATION: &[u8] = b"test_destination";
    const TEST_DEPOSIT_AMOUNT: u64 = 1000000;

    // Helper function to create a test operator keypair
    fn create_test_operator_keypair() -> (XOnlyPk, SecretKey) {
        let secp = Secp256k1::new();
        let secret_key = SecretKey::from_slice(&[1u8; 32]).unwrap();
        let keypair = bitcoin::secp256k1::Keypair::from_secret_key(&secp, &secret_key);
        let (xonly_pk, _) = keypair.x_only_public_key();
        let operators_pubkey =
            XOnlyPk::new(Buf32::new(xonly_pk.serialize())).expect("Valid public key");
        (operators_pubkey, secret_key)
    }

    // Helper function to create a test transaction using the standard test utility
    fn create_test_tx() -> Transaction {
        let (operators_pubkey, operators_privkey) = create_test_operator_keypair();
        create_test_deposit_tx(
            TEST_DEPOSIT_IDX,
            TEST_TAPSCRIPT_ROOT,
            TEST_DESTINATION,
            Amount::from_sat(TEST_DEPOSIT_AMOUNT),
            &operators_pubkey,
            &operators_privkey,
        )
    }

    #[test]
    fn test_validate_deposit_output_lock_success() {
        let (operators_pubkey, _) = create_test_operator_keypair();
        let tx = create_test_tx();

        // This should succeed
        let result = validate_deposit_output_lock(&tx, &operators_pubkey);
        assert!(
            result.is_ok(),
            "Valid deposit output lock should pass validation"
        );
    }

    #[test]
    fn test_validate_deposit_output_lock_missing_output() {
        let (operators_pubkey, _) = create_test_operator_keypair();
        let mut tx = create_test_tx();

        // Remove the deposit output (keep only OP_RETURN at index 0)
        tx.output.truncate(1);

        let result = validate_deposit_output_lock(&tx, &operators_pubkey);
        assert!(
            result.is_err(),
            "Should fail when deposit output is missing"
        );

        assert!(matches!(result, Err(DepositError::MissingOutput(_))));
        if let Err(DepositError::MissingOutput(index)) = result {
            assert_eq!(index, DEPOSIT_OUTPUT_INDEX);
        }
    }

    #[test]
    fn test_validate_deposit_output_lock_wrong_script() {
        use bitcoin::ScriptBuf;

        let (operators_pubkey, _) = create_test_operator_keypair();
        let mut tx = create_test_tx();

        // Mutate the deposit output to have wrong script (empty script instead of P2TR)
        tx.output[1].script_pubkey = ScriptBuf::new();

        let result = validate_deposit_output_lock(&tx, &operators_pubkey);
        assert!(
            result.is_err(),
            "Should fail when deposit output has wrong script"
        );

        assert!(matches!(result, Err(DepositError::InvalidSignature { .. })));
        if let Err(DepositError::InvalidSignature { reason }) = result {
            assert!(reason.contains("not locked to the aggregated operator key"));
        }
    }

    #[test]
    fn test_validate_drt_spending_signature_no_witness() {
        let (operators_pubkey, operators_privkey) = create_test_operator_keypair();
        let drt_tapnode_hash = Buf32::new(TEST_TAPSCRIPT_ROOT);

        // Create a signed transaction then remove the witness
        let mut tx = create_test_deposit_tx(
            TEST_DEPOSIT_IDX,
            TEST_TAPSCRIPT_ROOT,
            TEST_DESTINATION,
            Amount::from_sat(TEST_DEPOSIT_AMOUNT),
            &operators_pubkey,
            &operators_privkey,
        );

        // Clear the witness to test no witness case
        tx.input[0].witness.clear();

        let result = validate_drt_spending_signature(
            &tx,
            drt_tapnode_hash,
            &operators_pubkey,
            Amount::from_sat(TEST_DEPOSIT_AMOUNT),
        );

        assert!(
            result.is_err(),
            "Should fail when no witness data is present"
        );

        assert!(matches!(result, Err(DepositError::InvalidSignature { .. })));
        if let Err(DepositError::InvalidSignature { reason }) = result {
            assert!(reason.contains("No witness data found"));
        }
    }

    #[test]
    fn test_validate_drt_spending_signature_invalid_signature() {
        use bitcoin::Witness;

        let (operators_pubkey, operators_privkey) = create_test_operator_keypair();
        let drt_tapnode_hash = Buf32::new(TEST_TAPSCRIPT_ROOT);

        // Create a signed transaction then replace with invalid signature
        let mut tx = create_test_deposit_tx(
            TEST_DEPOSIT_IDX,
            TEST_TAPSCRIPT_ROOT,
            TEST_DESTINATION,
            Amount::from_sat(TEST_DEPOSIT_AMOUNT),
            &operators_pubkey,
            &operators_privkey,
        );

        // Replace with invalid witness data
        tx.input[0].witness = Witness::from_slice(&[&[0u8; 64]]); // Dummy signature

        let result = validate_drt_spending_signature(
            &tx,
            drt_tapnode_hash,
            &operators_pubkey,
            Amount::from_sat(TEST_DEPOSIT_AMOUNT),
        );

        assert!(result.is_err(), "Should fail with invalid signature");

        // The exact error depends on whether signature parsing or verification fails first
        assert!(matches!(result, Err(DepositError::InvalidSignature { .. })));
    }

    #[test]
    fn test_validate_drt_spending_signature_success() {
        // Create a specific keypair that we'll use consistently
        let secp = Secp256k1::new();
        let secret_key = SecretKey::from_slice(&[1u8; 32]).unwrap();
        let keypair = bitcoin::secp256k1::Keypair::from_secret_key(&secp, &secret_key);
        let (internal_key, _) = keypair.x_only_public_key();
        let operators_pubkey = XOnlyPk::new(Buf32::new(internal_key.serialize())).unwrap();

        let drt_tapnode_hash = Buf32::new(TEST_TAPSCRIPT_ROOT);

        // Create a properly signed transaction using the test utility
        let tx = create_test_deposit_tx(
            TEST_DEPOSIT_IDX,
            TEST_TAPSCRIPT_ROOT,
            TEST_DESTINATION,
            Amount::from_sat(TEST_DEPOSIT_AMOUNT),
            &operators_pubkey,
            &secret_key,
        );

        // Test the validation - this should succeed
        let result = validate_drt_spending_signature(
            &tx,
            drt_tapnode_hash,
            &operators_pubkey,
            Amount::from_sat(TEST_DEPOSIT_AMOUNT),
        );

        assert!(
            result.is_ok(),
            "Should succeed with valid signature: {result:?}"
        );
    }

    #[test]
    fn test_create_valid_p2tr_script() {
        use bitcoin::{ScriptBuf, XOnlyPublicKey};

        let (operators_pubkey, _) = create_test_operator_keypair();
        let secp = Secp256k1::new();

        let operators_xonly =
            XOnlyPublicKey::from_slice(operators_pubkey.inner().as_bytes()).unwrap();
        let script = ScriptBuf::new_p2tr(&secp, operators_xonly, None);

        // Verify it's a P2TR script
        assert!(script.is_p2tr(), "Generated script should be P2TR");
        assert_eq!(script.len(), 34, "P2TR script should be 34 bytes"); // OP_1 + 32 bytes
    }
}
