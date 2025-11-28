//! Minimal deposit transaction builders for testing

use bitcoin::{
    Amount, OutPoint, ScriptBuf, Sequence, TapNodeHash, TapSighashType, Transaction, TxIn, TxOut,
    Txid, Witness, XOnlyPublicKey,
    absolute::LockTime,
    hashes::Hash,
    secp256k1::Secp256k1,
    sighash::{Prevouts, SighashCache},
    taproot::TaprootBuilder,
    transaction::Version,
};
use strata_codec::encode_to_vec;
use strata_crypto::{
    EvenSecretKey,
    test_utils::schnorr::{create_agg_pubkey_from_privkeys, create_musig2_signature},
};
use strata_l1_txfmt::{ParseConfig, TagData, TagDataRef};

use crate::{
    constants::{BRIDGE_V1_SUBPROTOCOL_ID, DEPOSIT_TX_TYPE},
    deposit::DepositInfo,
    test_utils::TEST_MAGIC_BYTES,
};

/// Creates a test deposit transaction with MuSig2 signatures
///
/// Simple test helper that creates a fully signed deposit transaction for unit tests.
pub fn create_test_deposit_tx(
    deposit_info: &DepositInfo,
    operators_privkeys: &[EvenSecretKey],
) -> Transaction {
    // Create auxiliary data in the expected format for deposit transactions
    let aux_data = encode_to_vec(deposit_info.header_aux()).unwrap();
    let td = TagData::new(BRIDGE_V1_SUBPROTOCOL_ID, DEPOSIT_TX_TYPE, aux_data).unwrap();
    let sps_50_script = ParseConfig::new(*TEST_MAGIC_BYTES)
        .encode_script_buf(&td.as_ref())
        .unwrap();

    let secp = Secp256k1::new();
    let aggregated_xonly = create_agg_pubkey_from_privkeys(operators_privkeys);
    let deposit_script = ScriptBuf::new_p2tr(&secp, aggregated_xonly, None);

    let merkle_root =
        TapNodeHash::from_byte_array(deposit_info.header_aux().drt_tapscript_merkle_root());
    let drt_script_pubkey = ScriptBuf::new_p2tr(&secp, aggregated_xonly, Some(merkle_root));

    let deposit_amount: Amount = deposit_info.amt().into();
    let prev_txout = TxOut {
        value: deposit_amount,
        script_pubkey: drt_script_pubkey,
    };

    let unsigned_tx = Transaction {
        version: Version(2),
        lock_time: LockTime::ZERO,
        input: vec![TxIn {
            previous_output: OutPoint::null(),
            script_sig: ScriptBuf::new(),
            sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
            witness: Witness::new(),
        }],
        output: vec![
            TxOut {
                value: Amount::ZERO,
                script_pubkey: sps_50_script,
            },
            TxOut {
                value: deposit_amount,
                script_pubkey: deposit_script,
            },
        ],
    };

    // Sign with MuSig2
    let prevouts = [prev_txout];
    let prevouts_ref = Prevouts::All(&prevouts);
    let mut sighash_cache = SighashCache::new(&unsigned_tx);
    let sighash = sighash_cache
        .taproot_key_spend_signature_hash(0, &prevouts_ref, TapSighashType::Default)
        .unwrap();

    let msg = sighash.to_byte_array();
    let signature =
        create_musig2_signature(operators_privkeys, &msg, Some(merkle_root.to_byte_array()));

    Transaction {
        version: unsigned_tx.version,
        lock_time: unsigned_tx.lock_time,
        input: vec![TxIn {
            previous_output: OutPoint::null(),
            script_sig: ScriptBuf::new(),
            sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
            witness: Witness::from_slice(&[signature.serialize().as_slice()]),
        }],
        output: unsigned_tx.output,
    }
}

/// Builds an unsigned deposit transaction
///
/// This is the minimal core building logic. Takes clean parameters and constructs
/// the transaction structure. All parsing, signing, and error handling should be
/// done by the caller.
///
/// # Arguments
/// * `drt_txid` - The txid of the deposit request transaction
/// * `op_return_script` - The pre-built OP_RETURN script with metadata
/// * `agg_pubkey` - The aggregated operator public key
/// * `bridge_out_amount` - The amount for the bridge output
///
/// # Returns
/// The unsigned deposit transaction
pub fn build_deposit_transaction(
    drt_txid: Txid,
    op_return_script: ScriptBuf,
    agg_pubkey: XOnlyPublicKey,
    bridge_out_amount: Amount,
) -> Transaction {
    // Per spec: DRT output 1 is the P2TR deposit request output that we spend
    let tx_ins = vec![TxIn {
        previous_output: OutPoint::new(drt_txid, 1),
        script_sig: ScriptBuf::default(),
        sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
        witness: Witness::new(),
    }];

    // Build P2TR output for bridge
    let secp = Secp256k1::new();
    let taproot_builder = TaprootBuilder::new();
    let spend_info = taproot_builder
        .finalize(&secp, agg_pubkey)
        .expect("Taproot finalization cannot fail with no scripts");
    let merkle_root = spend_info.merkle_root();
    let bridge_address =
        bitcoin::Address::p2tr(&secp, agg_pubkey, merkle_root, bitcoin::Network::Regtest);

    let tx_outs = vec![
        TxOut {
            script_pubkey: op_return_script,
            value: Amount::ZERO,
        },
        TxOut {
            script_pubkey: bridge_address.script_pubkey(),
            value: bridge_out_amount,
        },
    ];

    Transaction {
        version: Version::TWO,
        lock_time: LockTime::ZERO,
        input: tx_ins,
        output: tx_outs,
    }
}

/// Creates the auxiliary data for a deposit transaction OP_RETURN
///
/// # Arguments
/// * `deposit_idx` - The deposit index
/// * `takeback_hash` - The taproot hash for the takeback script
/// * `ee_address` - The execution environment address
///
/// # Returns
/// The auxiliary data bytes that should be encoded into an OP_RETURN script
fn create_deposit_aux_data(
    deposit_idx: u32,
    takeback_hash: TapNodeHash,
    ee_address: &[u8],
) -> Vec<u8> {
    let mut aux_data = Vec::new();
    aux_data.extend_from_slice(&deposit_idx.to_be_bytes());
    aux_data.extend_from_slice(takeback_hash.as_ref());
    aux_data.extend_from_slice(ee_address);
    aux_data
}

/// Returns the subprotocol ID and transaction type for deposit transactions
fn deposit_tx_tag() -> (u8, u8) {
    (BRIDGE_V1_SUBPROTOCOL_ID, DEPOSIT_TX_TYPE)
}

/// Creates an OP_RETURN script for deposit transactions using SPS-50 format
///
/// This is the canonical function for creating deposit OP_RETURN scripts.
/// Both test utilities and production CLI should use this.
///
/// # Arguments
/// * `magic_bytes` - The magic bytes for the network
/// * `deposit_idx` - The deposit index
/// * `takeback_hash` - The taproot hash for the takeback script
/// * `ee_address` - The execution environment address
///
/// # Returns
/// The OP_RETURN script ready to be included in a transaction
///
/// # Errors
/// Returns an error if the SPS-50 encoding fails
pub fn create_deposit_op_return(
    magic_bytes: [u8; 4],
    deposit_idx: u32,
    takeback_hash: TapNodeHash,
    ee_address: &[u8],
) -> Result<ScriptBuf, String> {
    let aux_data = create_deposit_aux_data(deposit_idx, takeback_hash, ee_address);
    let (subprotocol_id, tx_type) = deposit_tx_tag();

    let tag_data = TagDataRef::new(subprotocol_id, tx_type, &aux_data)
        .map_err(|e| format!("SPS-50 format error: {}", e))?;

    let op_return_script = ParseConfig::new(magic_bytes)
        .encode_script_buf(&tag_data)
        .map_err(|e| format!("SPS-50 encoding error: {}", e))?;

    Ok(op_return_script)
}
