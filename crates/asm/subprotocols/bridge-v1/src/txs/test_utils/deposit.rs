//! Test utilities for creating deposit transactions.

use bitcoin::{
    Amount, ScriptBuf, Sequence, TapNodeHash, Transaction, TxIn, TxOut, Witness,
    absolute::LockTime,
    hashes::Hash,
    script::PushBytesBuf,
    secp256k1::{Secp256k1, SecretKey},
    sighash::{Prevouts, SighashCache, TapSighashType},
};
use strata_crypto::multisig::schemes::schnorr::create::{
    create_agg_pubkey_from_privkeys, create_musig2_signature,
};

use crate::txs::{deposit::DepositInfo, test_utils::TEST_MAGIC_BYTES};

// Creates a test deposit transaction with proper MuSig2 signatures following the SPS-50
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
