//! Test utilities for creating deposit transactions.

use bitcoin::{
    Amount, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Witness, TapNodeHash,
    absolute::LockTime, secp256k1::{Secp256k1, SecretKey, Keypair},
    key::TapTweak,
    sighash::{Prevouts, SighashCache, TapSighashType},
    hashes::Hash,
};

pub const TEST_MAGIC_BYTES: &[u8; 4] = b"ALPN";

/// Creates a test deposit transaction following the SPS-50 specification.
///
/// Creates a properly structured and signed Bitcoin deposit transaction with:
/// - Input 0: Spends a P2TR output from a Deposit Request Transaction (DRT) with valid signature
/// - Output 0: OP_RETURN containing SPS-50 tagged data
/// - Output 1: P2TR deposit output locked to the aggregated operator key
///
/// The transaction is created with a valid taproot signature that will pass
/// signature validation against the provided operator public key and tapscript root.
///
/// # Arguments
/// - `deposit_info`: The deposit information containing index, amount, destination, and tapscript root
/// - `operators_privkey`: The private key corresponding to the operator public key
///
/// # Returns
/// A properly formatted and signed Bitcoin transaction that can be parsed by ParseConfig
pub fn create_test_deposit_tx(
    deposit_info: &crate::txs::deposit::parse::DepositInfo,
    operators_privkey: &SecretKey,
) -> Transaction {
    use bitcoin::script::PushBytesBuf;
    use bitcoin::secp256k1::Message;

    use crate::constants::{BRIDGE_V1_SUBPROTOCOL_ID, DEPOSIT_TX_TYPE};

    // Create auxiliary data in the expected format for deposit transactions
    let mut aux_data = Vec::new();
    aux_data.extend_from_slice(&deposit_info.deposit_idx.to_be_bytes()); // 4 bytes
    aux_data.extend_from_slice(deposit_info.drt_tapnode_hash.as_ref()); // 32 bytes  
    aux_data.extend_from_slice(&deposit_info.address); // variable length

    // Create the complete SPS-50 tagged payload
    // Format: [MAGIC_BYTES][SUBPROTOCOL_ID][TX_TYPE][AUX_DATA]
    let mut tagged_payload = Vec::new();
    tagged_payload.extend_from_slice(TEST_MAGIC_BYTES); // 4 bytes magic
    tagged_payload.extend_from_slice(&BRIDGE_V1_SUBPROTOCOL_ID.to_be_bytes()); // 4 bytes subprotocol ID
    tagged_payload.extend_from_slice(&DEPOSIT_TX_TYPE.to_be_bytes()); // 4 bytes transaction type
    tagged_payload.extend_from_slice(&aux_data); // auxiliary data

    // Create P2TR script for deposit output locked to operators key (no merkle root)
    let secp = Secp256k1::new();
    let keypair = Keypair::from_secret_key(&secp, operators_privkey);
    let (operators_xonly, _) = keypair.x_only_public_key();
    let deposit_script = ScriptBuf::new_p2tr(&secp, operators_xonly, None);

    // Create the UTXO being spent (DRT output) with proper taproot script
    let merkle_root = TapNodeHash::from_byte_array(*deposit_info.drt_tapnode_hash.as_ref());
    let drt_script_pubkey = ScriptBuf::new_p2tr(&secp, operators_xonly, Some(merkle_root));
    
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
    
    // Create the signature using the unsigned transaction
    let prevtxouts = [prev_txout];
    let prevouts = Prevouts::All(&prevtxouts);
    let sighash = SighashCache::new(&unsigned_tx)
        .taproot_key_spend_signature_hash(0, &prevouts, TapSighashType::Default)
        .unwrap();
    
    let msg = Message::from_digest(sighash.to_byte_array());
    
    // Create the tweaked keypair for signing
    let tweaked_keypair = keypair.tap_tweak(&secp, Some(merkle_root));
    let signature = secp.sign_schnorr(&msg, &tweaked_keypair.to_keypair());
    
    // Create the final signed transaction (reuse outputs from unsigned_tx)
    Transaction {
        version: unsigned_tx.version,
        lock_time: unsigned_tx.lock_time,
        input: vec![TxIn {
            previous_output: bitcoin::OutPoint::null(),
            script_sig: ScriptBuf::new(),
            sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
            witness: Witness::from_slice(&[signature.as_ref()]),
        }],
        output: unsigned_tx.output,
    }
}
