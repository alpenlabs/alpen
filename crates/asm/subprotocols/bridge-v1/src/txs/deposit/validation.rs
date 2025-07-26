use bitcoin::{
    Amount, ScriptBuf, TapNodeHash, Transaction, TxOut, XOnlyPublicKey,
    hashes::Hash,
    key::TapTweak,
    sighash::{Prevouts, SighashCache},
    taproot::{self},
};
use secp256k1::Message;
use strata_primitives::{buf::Buf32, l1::XOnlyPk};

use crate::errors::DepositError;

const DEPOSIT_OUTPUT_INDEX: u32 = 1;

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
