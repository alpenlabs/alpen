//! Minimal deposit transaction builders for testing

use bitcoin::{
    Address, Amount, OutPoint, ScriptBuf, Sequence, TapNodeHash, TapSighashType, Transaction, TxIn,
    TxOut, Txid, Witness, XOnlyPublicKey,
    absolute::LockTime,
    constants::COINBASE_MATURITY,
    hashes::Hash,
    secp256k1::Secp256k1,
    sighash::{Prevouts, SighashCache},
    taproot::TaprootBuilder,
    transaction::Version,
};
use strata_crypto::{
    EvenSecretKey,
    test_utils::schnorr::{Musig2Tweak, create_agg_pubkey_from_privkeys, create_musig2_signature},
};
use strata_l1_txfmt::ParseConfig;
use strata_test_utils_btcio::{
    address::derive_musig2_p2tr_address, get_bitcoind_and_client, mining::mine_blocks_blocking,
    submit::submit_transaction_with_keys_blocking,
};

use crate::{
    deposit::{DepositInfo, DepositTxHeaderAux},
    deposit_request::DrtHeaderAux,
    test_utils::{TEST_MAGIC_BYTES, create_dummy_tx, create_test_deposit_request_tx},
};

/// Creates a test deposit transaction with MuSig2 signatures
///
/// Simple test helper that creates a fully signed deposit transaction for unit tests.
pub fn create_test_deposit_tx(
    deposit_info: &DepositInfo,
    operators_privkeys: &[EvenSecretKey],
) -> Transaction {
    // Create auxiliary data in the expected format for deposit transactions
    let td = deposit_info.header_aux().build_tag_data().unwrap();
    let sps50_script = ParseConfig::new(*TEST_MAGIC_BYTES)
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

    let mut unsigned_tx = create_dummy_tx(1, 2);
    unsigned_tx.output[0].script_pubkey = sps50_script;
    unsigned_tx.output[1].script_pubkey = deposit_script;
    unsigned_tx.output[1].value = deposit_amount;

    // Sign with MuSig2
    let prevouts = [prev_txout];
    let prevouts_ref = Prevouts::All(&prevouts);
    let mut sighash_cache = SighashCache::new(&unsigned_tx);
    let sighash = sighash_cache
        .taproot_key_spend_signature_hash(0, &prevouts_ref, TapSighashType::Default)
        .unwrap();

    let msg = sighash.to_byte_array();
    let signature = create_musig2_signature(
        operators_privkeys,
        &msg,
        Musig2Tweak::TaprootScript(merkle_root.to_byte_array()),
    );

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

fn create_test_deposit_tx_new(
    dt_header_aux: DepositTxHeaderAux,
    nn_address: Address,
) -> Transaction {
    let mut tx = create_dummy_tx(1, 2);

    let tag = dt_header_aux.build_tag_data().unwrap();
    let sps_50_script = ParseConfig::new(*TEST_MAGIC_BYTES)
        .encode_script_buf(&tag.as_ref())
        .unwrap();

    tx.output[0].script_pubkey = sps_50_script;
    tx.output[1].script_pubkey = nn_address.script_pubkey();
    tx.output[1].value = Amount::from_sat(10_000); // dust

    tx
}

pub fn create_connected_drt_and_dt(
    drt_header_aux: DrtHeaderAux,
    dt_header_aux: DepositTxHeaderAux,
    operator_keys: &[EvenSecretKey],
) -> (Transaction, Transaction) {
    let (bitcoind, client) = get_bitcoind_and_client();
    let _ =
        mine_blocks_blocking(&bitcoind, &client, (COINBASE_MATURITY + 1) as usize, None).unwrap();

    let (nn_address, internal_key) = derive_musig2_p2tr_address(operator_keys).unwrap();
    let mut drt = create_test_deposit_request_tx(&drt_header_aux, internal_key);

    let drt_txid =
        submit_transaction_with_keys_blocking(&bitcoind, &client, operator_keys, &mut drt).unwrap();

    let mut dt = create_test_deposit_tx_new(dt_header_aux, nn_address);
    dt.input[0].previous_output = OutPoint {
        txid: drt_txid,
        vout: 1,
    };

    let _ =
        submit_transaction_with_keys_blocking(&bitcoind, &client, operator_keys, &mut dt).unwrap();

    (drt, dt)
}
