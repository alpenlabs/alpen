//! Test utilities for creating deposit transactions.

use bitcoin::{
    Amount, ScriptBuf, Sequence, TapNodeHash, Transaction, TxIn, TxOut, Witness, XOnlyPublicKey,
    absolute::LockTime,
    hashes::Hash,
    script::PushBytesBuf,
    secp256k1::{PublicKey, Secp256k1, SecretKey},
    sighash::{Prevouts, SighashCache, TapSighashType},
};
use musig2::{FirstRound, KeyAggContext, SecNonceSpices};
use rand::{RngCore, rngs::OsRng};
use strata_crypto::multisig::aggregate_schnorr_keys;
use strata_primitives::{buf::Buf32, crypto::EvenSecretKey};

use crate::txs::{deposit::DepositInfo, test_utils::TEST_MAGIC_BYTES};

/// Creates a MuSig2 signature from multiple operators.
///
/// This function simulates the MuSig2 signing process where multiple operators
/// coordinate to create a single aggregated signature.
///
/// # Arguments
/// - `operators_privkeys`: Private keys of all operators participating in signing
/// - `message`: The message to be signed (typically a sighash)
/// - `tweak`: Optional tweak for taproot spending (merkle root)
///
/// # Returns
/// The aggregated MuSig2 signature
fn create_musig2_signature(
    operators_privkeys: &[SecretKey],
    message: &[u8; 32],
    tweak: Option<[u8; 32]>,
) -> musig2::CompactSignature {
    let secp = Secp256k1::new();

    // Adjust both public keys and private keys for even parity
    let adjusted_keys: Vec<(PublicKey, SecretKey)> = operators_privkeys
        .iter()
        .map(|sk| {
            let even_sk = EvenSecretKey::from(*sk);
            let pk = PublicKey::from_secret_key(&secp, &even_sk);
            (pk, *even_sk.as_ref())
        })
        .collect();

    // Create KeyAggContext with even parity public keys
    let mut key_agg_ctx =
        KeyAggContext::new(adjusted_keys.iter().map(|(pk, _)| *pk).collect::<Vec<_>>())
            .expect("failed to create KeyAggContext");

    // Apply tweak if provided (for taproot spending)
    if let Some(tweak) = tweak {
        key_agg_ctx = key_agg_ctx
            .with_taproot_tweak(&tweak)
            .expect("Failed to apply taproot tweak to key aggregation context");
    }

    let mut first_rounds = Vec::new();
    let mut public_nonces = Vec::new();

    // Phase 1: Generate nonces for each signer
    for (signer_index, (_, adjusted_privkey)) in adjusted_keys.iter().enumerate() {
        // Generate secure random nonce seed for each signer
        let mut nonce_seed = [0u8; 32];
        OsRng.fill_bytes(&mut nonce_seed);

        let first_round = FirstRound::new(
            key_agg_ctx.clone(),
            nonce_seed,
            signer_index,
            SecNonceSpices::new()
                .with_seckey(*adjusted_privkey)
                .with_message(message),
        )
        .expect("Failed to create FirstRound");

        public_nonces.push(first_round.our_public_nonce());
        first_rounds.push(first_round);
    }

    // Phase 2: Exchange nonces and create partial signatures
    let mut second_rounds = Vec::new();
    for (signer_index, mut first_round) in first_rounds.into_iter().enumerate() {
        // Each signer receives nonces from all other signers
        for (other_index, public_nonce) in public_nonces.iter().enumerate() {
            if other_index != signer_index {
                first_round
                    .receive_nonce(other_index, public_nonce.clone())
                    .expect("Failed to receive nonce");
            }
        }

        // Finalize first round to create second round
        let second_round = first_round
            .finalize(adjusted_keys[signer_index].1, *message)
            .expect("Failed to finalize first round");

        second_rounds.push(second_round);
    }

    // Phase 3: Exchange partial signatures
    let partial_signatures: Vec<musig2::PartialSignature> = second_rounds
        .iter()
        .map(|round| round.our_signature::<musig2::PartialSignature>())
        .collect();

    // Use the first signer to finalize (any signer can do this)
    if let Some((signer_index, mut second_round)) = second_rounds.into_iter().enumerate().next() {
        for (other_index, partial_sig) in partial_signatures.iter().enumerate() {
            if other_index != signer_index {
                second_round
                    .receive_signature(other_index, *partial_sig)
                    .expect("Failed to receive partial signature");
            }
        }

        // Finalize to get the aggregated signature
        return second_round
            .finalize()
            .expect("Failed to finalize MuSig2 signature");
    }

    panic!("No signers available to finalize signature");
}

/// Creates a test deposit transaction with proper MuSig2 signatures following the SPS-50
/// specification.
///
/// Creates a properly structured and signed Bitcoin deposit transaction with:
/// - Input 0: Spends a P2TR output from a Deposit Request Transaction (DRT) with valid MuSig2
///   signature
/// - Output 0: OP_RETURN containing SPS-50 tagged data
/// - Output 1: P2TR deposit output locked to the aggregated operator key
///
/// The transaction is created with a valid MuSig2 signature that will pass
/// signature validation against the aggregated operator public key.
///
/// # Arguments
/// - `deposit_info`: The deposit information containing index, amount, destination, and tapscript
///   root
/// - `operators_privkeys`: Slice of private keys corresponding to the operator public keys
///
/// # Returns
/// The properly formatted and signed Bitcoin transaction
pub(crate) fn create_test_deposit_tx(
    deposit_info: &DepositInfo,
    operators_privkeys: &[SecretKey],
) -> Transaction {
    use crate::constants::{BRIDGE_V1_SUBPROTOCOL_ID, DEPOSIT_TX_TYPE};

    // Create auxiliary data in the expected format for deposit transactions
    let mut aux_data = Vec::new();
    aux_data.extend_from_slice(&deposit_info.deposit_idx.to_be_bytes()); // 4 bytes
    aux_data.extend_from_slice(deposit_info.drt_tapscript_merkle_root.as_ref()); // 32 bytes  
    aux_data.extend_from_slice(&deposit_info.address); // variable length

    // Create the complete SPS-50 tagged payload
    // Format: [MAGIC_BYTES][SUBPROTOCOL_ID][TX_TYPE][AUX_DATA]
    let mut tagged_payload = Vec::new();
    tagged_payload.extend_from_slice(TEST_MAGIC_BYTES); // 4 bytes magic
    tagged_payload.extend_from_slice(&BRIDGE_V1_SUBPROTOCOL_ID.to_be_bytes()); // 4 bytes subprotocol ID
    tagged_payload.extend_from_slice(&DEPOSIT_TX_TYPE.to_be_bytes()); // 4 bytes transaction type
    tagged_payload.extend_from_slice(&aux_data); // auxiliary data

    let secp = Secp256k1::new();

    // Create MuSig2 context for signing (using PublicKey format)
    let aggregated_xonly = create_agg_pubkey_from_privkeys(operators_privkeys);

    // Use aggregated key for deposit output (matches validation expectation)
    let deposit_script = ScriptBuf::new_p2tr(&secp, aggregated_xonly, None);

    // Create the UTXO being spent (DRT output) with aggregated key for MuSig2
    let merkle_root =
        TapNodeHash::from_byte_array(*deposit_info.drt_tapscript_merkle_root.as_ref());
    let drt_script_pubkey = ScriptBuf::new_p2tr(&secp, aggregated_xonly, Some(merkle_root));

    let deposit_amount: Amount = deposit_info.amt.into();
    let prev_txout = TxOut {
        value: deposit_amount,
        script_pubkey: drt_script_pubkey,
    };

    // Create the transaction structure first (without signature)
    let unsigned_tx = Transaction {
        version: bitcoin::transaction::Version(2),
        lock_time: LockTime::ZERO,
        input: vec![TxIn {
            previous_output: bitcoin::OutPoint::null(),
            script_sig: ScriptBuf::new(),
            sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
            witness: Witness::new(),
        }],
        output: vec![
            // OP_RETURN output at index 0 (contains the SPS-50 tagged data)
            TxOut {
                value: Amount::ZERO,
                script_pubkey: ScriptBuf::new_op_return(
                    PushBytesBuf::try_from(tagged_payload).unwrap(),
                ),
            },
            // Deposit output at index 1 (P2TR locked to aggregated operator key)
            TxOut {
                value: deposit_amount,
                script_pubkey: deposit_script,
            },
        ],
    };

    // Create proper MuSig2 signature
    // Compute sighash for taproot key-spend signature
    let prevtxouts = [prev_txout];
    let prevouts = Prevouts::All(&prevtxouts);
    let mut sighash_cache = SighashCache::new(&unsigned_tx);
    let sighash = sighash_cache
        .taproot_key_spend_signature_hash(0, &prevouts, TapSighashType::Default)
        .unwrap();

    let msg = sighash.to_byte_array();

    // Create MuSig2 signature using all operators
    let final_signature =
        create_musig2_signature(operators_privkeys, &msg, Some(merkle_root.to_byte_array()));

    // Create the final signed transaction
    Transaction {
        version: unsigned_tx.version,
        lock_time: unsigned_tx.lock_time,
        input: vec![TxIn {
            previous_output: bitcoin::OutPoint::null(),
            script_sig: ScriptBuf::new(),
            sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
            witness: Witness::from_slice(&[final_signature.serialize().as_slice()]),
        }],
        output: unsigned_tx.output,
    }
}

fn create_agg_pubkey_from_privkeys(operators_privkeys: &[SecretKey]) -> XOnlyPublicKey {
    let pubkeys: Vec<_> = operators_privkeys
        .iter()
        .map(|sk| PublicKey::from_secret_key(&Secp256k1::new(), sk))
        .map(|pk| pk.x_only_public_key().0)
        .map(|xpk| Buf32::from(xpk.serialize()))
        .collect();
    aggregate_schnorr_keys(pubkeys.iter()).expect("generation of aggregated public key failed")
}

#[cfg(test)]
mod tests {
    use bitcoin::{
        key::TapTweak,
        secp256k1::{Secp256k1, SecretKey},
    };
    use rand::rngs::OsRng;

    use super::*;

    #[test]
    fn test_musig2_signature_validation() {
        let secp = Secp256k1::new();

        // Test message to sign - use random message
        let mut message = [0u8; 32];
        OsRng.fill_bytes(&mut message);

        // Test with tweak (taproot spending) - use random tweak
        let mut tweak = [0u8; 32];
        OsRng.fill_bytes(&mut tweak);

        // Generate test private keys for 3 operators
        let operator_privkeys: Vec<SecretKey> =
            (0..3).map(|_| SecretKey::new(&mut OsRng)).collect();

        // Test without tweak
        let signature_no_tweak = create_musig2_signature(&operator_privkeys, &message, None);

        let signature_with_tweak =
            create_musig2_signature(&operator_privkeys, &message, Some(tweak));

        // Signatures should be different due to different tweaks
        assert_ne!(
            signature_no_tweak.serialize(),
            signature_with_tweak.serialize()
        );

        let agg_pubkey_no_tweak = create_agg_pubkey_from_privkeys(&operator_privkeys);
        let agg_pubkey_with_tweak = agg_pubkey_no_tweak
            .tap_tweak(&secp, Some(TapNodeHash::from_byte_array(tweak)))
            .0
            .to_x_only_public_key();

        // Verify signature without tweak
        let verification_result = secp.verify_schnorr(
            &bitcoin::secp256k1::schnorr::Signature::from_slice(&signature_no_tweak.serialize())
                .expect("Valid signature"),
            &bitcoin::secp256k1::Message::from_digest(message),
            &agg_pubkey_no_tweak,
        );
        assert!(verification_result.is_ok());

        // Verify signature with tweak
        let tweaked_verification_result = secp.verify_schnorr(
            &bitcoin::secp256k1::schnorr::Signature::from_slice(&signature_with_tweak.serialize())
                .expect("Valid signature"),
            &bitcoin::secp256k1::Message::from_digest(message),
            &agg_pubkey_with_tweak,
        );
        assert!(tweaked_verification_result.is_ok());
    }
}
