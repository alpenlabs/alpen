use bitcoin::{
    Amount, ScriptBuf, TapNodeHash, Transaction, TxOut, XOnlyPublicKey,
    hashes::Hash,
    key::TapTweak,
    sighash::{Prevouts, SighashCache},
    taproot::{self},
};
use secp256k1::Message;
use strata_primitives::{buf::Buf32, l1::XOnlyPk};

use crate::{
    deposit::DEPOSIT_OUTPUT_INDEX,
    errors::{DepositOutputError, DrtSignatureError},
};

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
/// - `Err(DrtSignatureError)` - If signature validation fails with specific details about the
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
) -> Result<(), DrtSignatureError> {
    // Initialize necessary variables and dependencies
    let secp = secp256k1::SECP256K1;

    // FIXME: Use latest version of `bitcoin` once released. The underlying
    // `bitcoinconsensus==0.106` will have support for taproot validation. So here, we just need
    // to create TxOut from operator pubkeys and tapnode hash and call `tx.verify()`.

    // Extract and validate input signature
    let input = tx.input[0].clone();

    // Check if witness is present.
    if input.witness.is_empty() {
        return Err(DrtSignatureError::MissingWitness);
    }
    let sig_witness = &input.witness[0];

    // rust-bitcoin taproot::Signature handles both both 64-byte (SIGHASH_DEFAULT)
    // and 65-byte (explicit sighash) signatures.
    let taproot_sig = taproot::Signature::from_slice(sig_witness)
        .map_err(|e| DrtSignatureError::InvalidSignatureFormat(e.to_string()))?;
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
        .map_err(|e| DrtSignatureError::SchnorrVerificationFailed(e.to_string()))?;

    Ok(())
}

/// Validates that the deposit output is locked to the N/N aggregated operator key.
///
/// This function verifies that the deposit output at `DEPOSIT_OUTPUT_INDEX` is a P2TR
/// output locked to the provided aggregated operator public key with no merkle root
/// (key-spend only). This ensures the deposited funds can only be spent by the N/N
/// operator set.
///
/// # Panics
///
/// This function will panic if the deposit output at `DEPOSIT_OUTPUT_INDEX` is missing.
/// This is safe because this validation is only called on transactions that have already
/// been parsed and verified to have the proper deposit transaction structure.
///
/// # Parameters
///
/// - `tx` - The deposit transaction to validate
/// - `operators_agg_pubkey` - The aggregated operator public key that should control the deposit
///
/// # Returns
///
/// - `Ok(())` - If the deposit output is properly locked to the operator key
/// - `Err(DepositOutputError)` - If the output has wrong script type or wrong key
pub fn validate_deposit_output_lock(
    tx: &Transaction,
    operators_agg_pubkey: &XOnlyPk,
) -> Result<(), DepositOutputError> {
    // Get the deposit output at the expected index.
    // This expect is safe because we only validate transactions that have already been
    // parsed and confirmed to have proper deposit transaction structure. If there's no
    // deposit output at this index, it means the transaction wasn't properly validated
    // during parsing, which indicates a programming error.
    let deposit_output = tx
        .output
        .get(DEPOSIT_OUTPUT_INDEX as usize)
        .expect("invalid deposit tx structure");

    // Extract the internal key from the P2TR script
    let secp = secp256k1::SECP256K1;
    let operators_pubkey = XOnlyPublicKey::from_slice(operators_agg_pubkey.inner().as_bytes())
        .map_err(|_| DepositOutputError::InvalidOperatorKey)?;

    // Create expected P2TR script with no merkle root (key-spend only)
    let expected_script = ScriptBuf::new_p2tr(secp, operators_pubkey, None);

    // Verify the deposit output script matches the expected P2TR script
    if deposit_output.script_pubkey != expected_script {
        return Err(DepositOutputError::WrongOutputLock);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use bitcoin::{
        Witness,
        secp256k1::{PublicKey, Secp256k1, SecretKey},
    };
    use musig2::KeyAggContext;
    use rand::Rng;
    use strata_crypto::EvenSecretKey;
    use strata_primitives::{buf::Buf32, l1::XOnlyPk};
    use strata_test_utils::ArbitraryGenerator;

    use super::*;
    use crate::{deposit::parse::DepositInfo, test_utils::create_test_deposit_tx};

    // Helper function to create test operator keys with proper MuSig2 aggregation
    fn create_test_operators() -> (XOnlyPk, Vec<EvenSecretKey>) {
        let secp = Secp256k1::new();
        let mut rng = secp256k1::rand::thread_rng();
        let num_operators = rng.gen_range(2..=5);

        // Generate random operator keys
        let operators_privkeys: Vec<EvenSecretKey> = (0..num_operators)
            .map(|_| SecretKey::new(&mut rng).into())
            .collect();

        // Create MuSig2 context for consistent key aggregation (same as create_test_deposit_tx)
        let pubkeys: Vec<PublicKey> = operators_privkeys
            .iter()
            .map(|sk| PublicKey::from_secret_key(&secp, sk))
            .map(|pk| {
                PublicKey::from_x_only_public_key(pk.x_only_public_key().0, secp256k1::Parity::Even)
            })
            .collect();

        let key_agg_ctx = KeyAggContext::new(pubkeys).expect("Failed to create KeyAggContext");

        // Use MuSig2 aggregated key to ensure consistency with create_test_deposit_tx
        let aggregated_xonly: bitcoin::secp256k1::XOnlyPublicKey = key_agg_ctx.aggregated_pubkey();
        let operators_pubkey = XOnlyPk::new(Buf32::new(aggregated_xonly.serialize()))
            .expect("Valid aggregated public key");

        (operators_pubkey, operators_privkeys)
    }

    // Helper function to create a test transaction and return both tx and aggregated pubkey
    fn create_test_tx_with_agg_pubkey() -> (Transaction, XOnlyPk) {
        let (operators_pubkey, operators_privkeys) = create_test_operators();
        let deposit_info: DepositInfo = ArbitraryGenerator::new().generate();
        let tx = create_test_deposit_tx(&deposit_info, &operators_privkeys);
        (tx, operators_pubkey)
    }

    #[test]
    fn test_validate_deposit_output_lock_success() {
        let (tx, operators_pubkey) = create_test_tx_with_agg_pubkey();

        // This should succeed
        let result = validate_deposit_output_lock(&tx, &operators_pubkey);
        assert!(
            result.is_ok(),
            "Valid deposit output lock should pass validation"
        );
    }

    #[test]
    fn test_validate_deposit_output_lock_missing_output() {
        let (mut tx, operators_pubkey) = create_test_tx_with_agg_pubkey();

        // Remove the deposit output (keep only OP_RETURN at index 0)
        tx.output.truncate(1);

        // This should panic since we removed the deposit output
        let err = validate_deposit_output_lock(&tx, &operators_pubkey).unwrap_err();
        assert!(matches!(err, DepositOutputError::WrongOutputLock))
    }

    #[test]
    fn test_validate_deposit_output_lock_wrong_script() {
        use bitcoin::ScriptBuf;

        let (mut tx, operators_pubkey) = create_test_tx_with_agg_pubkey();

        // Mutate the deposit output to have wrong script (empty script instead of P2TR)
        tx.output[1].script_pubkey = ScriptBuf::new();

        let result = validate_deposit_output_lock(&tx, &operators_pubkey);
        assert!(
            result.is_err(),
            "Should fail when deposit output has wrong script"
        );

        assert!(matches!(result, Err(DepositOutputError::WrongOutputLock)));
    }

    #[test]
    fn test_validate_drt_spending_signature_no_witness() {
        let (operators_pubkey, operators_privkeys) = create_test_operators();

        // Create a signed transaction then remove the witness
        let deposit_info: DepositInfo = ArbitraryGenerator::new().generate();
        let mut tx = create_test_deposit_tx(&deposit_info, &operators_privkeys);

        // Clear the witness to test no witness case
        tx.input[0].witness.clear();

        let result = validate_drt_spending_signature(
            &tx,
            deposit_info.drt_tapscript_merkle_root,
            &operators_pubkey,
            deposit_info.amt.into(),
        );

        assert!(
            result.is_err(),
            "Should fail when no witness data is present"
        );

        assert!(matches!(result, Err(DrtSignatureError::MissingWitness)));
    }

    #[test]
    fn test_validate_drt_spending_signature_invalid_signature_format() {
        let (operators_pubkey, operators_privkeys) = create_test_operators();

        // Create a signed transaction then replace with invalid signature
        let deposit_info: DepositInfo = ArbitraryGenerator::new().generate();
        let mut tx = create_test_deposit_tx(&deposit_info, &operators_privkeys);

        // Replace with invalid witness data
        tx.input[0].witness = Witness::from_slice(&[&[0u8; 66]]); // larger sig len

        let err = validate_drt_spending_signature(
            &tx,
            deposit_info.drt_tapscript_merkle_root,
            &operators_pubkey,
            deposit_info.amt.into(),
        )
        .unwrap_err();

        assert!(matches!(err, DrtSignatureError::InvalidSignatureFormat(_)));

        // Replace with invalid witness data
        tx.input[0].witness = Witness::from_slice(&[&[0u8; 32]]); // smaller sig len

        let err = validate_drt_spending_signature(
            &tx,
            deposit_info.drt_tapscript_merkle_root,
            &operators_pubkey,
            deposit_info.amt.into(),
        )
        .unwrap_err();

        assert!(matches!(err, DrtSignatureError::InvalidSignatureFormat(_)));
    }

    #[test]
    fn test_validate_drt_spending_signature_invalid_signature() {
        let (operators_pubkey, operators_privkeys) = create_test_operators();

        // Create a signed transaction then replace with invalid signature
        let deposit_info: DepositInfo = ArbitraryGenerator::new().generate();
        let mut tx = create_test_deposit_tx(&deposit_info, &operators_privkeys);

        // Replace with invalid witness data
        tx.input[0].witness = Witness::from_slice(&[&[0u8; 64]]);

        let err = validate_drt_spending_signature(
            &tx,
            deposit_info.drt_tapscript_merkle_root,
            &operators_pubkey,
            deposit_info.amt.into(),
        )
        .unwrap_err();

        assert!(matches!(
            err,
            DrtSignatureError::SchnorrVerificationFailed(_)
        ));
    }

    #[test]
    fn test_validate_drt_spending_signature_success() {
        // Create deposit info and transaction with consistent parameters
        let deposit_info: DepositInfo = ArbitraryGenerator::new().generate();
        let (operators_pubkey, operators_privkeys) = create_test_operators();
        let tx = create_test_deposit_tx(&deposit_info, &operators_privkeys);

        // Test the validation using the same tapnode hash from deposit_info
        let result = validate_drt_spending_signature(
            &tx,
            deposit_info.drt_tapscript_merkle_root,
            &operators_pubkey,
            deposit_info.amt.into(),
        );

        assert!(result.is_ok(), "Valid signature should pass validation");
    }

    #[test]
    fn test_create_valid_p2tr_script() {
        let (operators_pubkey, _) = create_test_operators();
        let secp = Secp256k1::new();

        let operators_xonly =
            XOnlyPublicKey::from_slice(operators_pubkey.inner().as_bytes()).unwrap();
        let script = ScriptBuf::new_p2tr(&secp, operators_xonly, None);

        // Verify it's a P2TR script
        assert!(script.is_p2tr(), "Generated script should be P2TR");
        assert_eq!(script.len(), 34, "P2TR script should be 34 bytes"); // OP_1 + 32 bytes
    }
}
