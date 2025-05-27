use std::collections::{BTreeMap, HashSet};

use arbitrary::{Arbitrary, Unstructured};
use bitcoin::{
    absolute::LockTime,
    opcodes::all::{OP_PUSHNUM_1, OP_RETURN},
    script::Builder,
    secp256k1::{Keypair, PublicKey, SecretKey, SECP256K1},
    taproot::{LeafVersion, TaprootBuilder, TaprootSpendInfo},
    transaction::Version,
    Address, Amount, Network, Psbt, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Witness,
};
use musig2::{KeyAggContext, SecNonce};
use rand::{rngs::OsRng, seq::SliceRandom, RngCore};
use strata_primitives::{
    bitcoin_bosd,
    bridge::{OperatorIdx, PublickeyTable, TxSigningData},
    l1::{BitcoinPsbt, BitcoinTxOut, OutputRef, TaprootSpendPath},
};

/// Generate `count` (public key, private key) pairs as two separate [`Vec`].
pub fn generate_keypairs(count: usize) -> (Vec<PublicKey>, Vec<SecretKey>) {
    let mut secret_keys: Vec<SecretKey> = Vec::with_capacity(count);
    let mut pubkeys: Vec<PublicKey> = Vec::with_capacity(count);

    let mut pubkeys_set: HashSet<PublicKey> = HashSet::new();

    while pubkeys_set.len() != count {
        let sk = SecretKey::new(&mut OsRng);
        let keypair = Keypair::from_secret_key(SECP256K1, &sk);
        let pubkey = PublicKey::from_keypair(&keypair);

        if pubkeys_set.insert(pubkey) {
            secret_keys.push(sk);
            pubkeys.push(pubkey);
        }
    }

    (pubkeys, secret_keys)
}

pub fn generate_pubkey_table(table: &[PublicKey]) -> PublickeyTable {
    let pubkey_table = table
        .iter()
        .enumerate()
        .map(|(i, pk)| (i as OperatorIdx, *pk))
        .collect::<BTreeMap<OperatorIdx, PublicKey>>();

    PublickeyTable::from(pubkey_table)
}

/// Generate an arbitrary prevout.
pub fn generate_mock_prevouts() -> TxOut {
    let data = &[0u8; 1024];
    let mut unstructured = Unstructured::new(&data[..]);
    let prevout = BitcoinTxOut::arbitrary(&mut unstructured).unwrap();

    prevout.inner().clone()
}

/// Generate a mock unsigned tx with two scripts.
///
/// An unsigned tx has an empty script_sig/witness fields.
///
/// # Returns
///
/// A tuple containing:
///
/// 1) The created unsigned [`Transaction`].
/// 2) The [`TaprootSpendInfo`] to spend via a [`ScriptBuf`].
/// 3) The [`ScriptBuf`] that can be spent.
pub fn generate_mock_unsigned_tx() -> (Transaction, TaprootSpendInfo, ScriptBuf) {
    // actually construct a valid taptree order to check PSBT finalization
    let (pks, _) = generate_keypairs(1);
    let internal_key = pks[0].x_only_public_key().0;

    let anyone_can_spend = Builder::new().push_opcode(OP_PUSHNUM_1).into_script();
    let none_can_spend = Builder::new().push_opcode(OP_RETURN).into_script();

    let taproot = TaprootBuilder::new()
        .add_leaf(1, anyone_can_spend.clone())
        .expect("should be able to add tapleaf")
        .add_leaf(1, none_can_spend.clone())
        .expect("should be able to add tapleaf");

    let spend_info = taproot
        .finalize(SECP256K1, internal_key)
        .expect("taproot build should work");
    let merkle_root = spend_info.merkle_root();

    let address = Address::p2tr(SECP256K1, internal_key, merkle_root, Network::Regtest);

    let random_bytes = vec![0u8; 1024];
    let mut unstructured = Unstructured::new(&random_bytes);
    let previous_output = *OutputRef::arbitrary(&mut unstructured)
        .expect("should be able to generate arbitrary output ref")
        .outpoint();

    let tx = Transaction {
        version: Version(2),
        lock_time: LockTime::ZERO,
        input: vec![TxIn {
            previous_output,
            script_sig: Default::default(),
            sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
            witness: Witness::new(),
        }],
        output: vec![TxOut {
            value: Amount::from_sat(0), // so that we can add many inputs above
            script_pubkey: address.script_pubkey(),
        }],
    };

    (tx, spend_info, anyone_can_spend)
}

/// Generate a mock spend info with arbitrary data.
pub fn generate_mock_spend_info() -> TaprootSpendPath {
    let data = &[0u8; 1024];
    let mut unstructured = Unstructured::new(&data[..]);
    let spend_info: TaprootSpendPath = TaprootSpendPath::arbitrary(&mut unstructured).unwrap();

    spend_info
}

/// Create mock [`TxSigningData`].
pub fn generate_mock_tx_signing_data(keys_spend_only: bool) -> TxSigningData {
    // Create a minimal unsigned transaction
    let (unsigned_tx, spend_info, script_buf) = generate_mock_unsigned_tx();
    let prevout = generate_mock_prevouts();

    let spend_path = if keys_spend_only {
        TaprootSpendPath::Key
    } else {
        TaprootSpendPath::Script {
            script_buf: script_buf.clone(),
            control_block: spend_info
                .control_block(&(script_buf, LeafVersion::TapScript))
                .expect("should be able to construct control block"),
        }
    };

    let mut psbt = Psbt::from_unsigned_tx(unsigned_tx).expect("should be able to create psbt");
    let input = psbt.inputs.first_mut().expect("input should exist in psbt");
    input.witness_utxo = Some(prevout);

    let psbt = BitcoinPsbt::from(psbt);

    TxSigningData { psbt, spend_path }
}

/// Generate a MuSig2 sec nonce.
pub fn generate_sec_nonce(
    msg: &impl AsRef<[u8]>,
    pubkeys: impl IntoIterator<Item = PublicKey>,
    seckey: SecretKey,
    tweak: bool,
) -> SecNonce {
    let key_agg_ctx = KeyAggContext::new(pubkeys).expect("key agg context should be created");
    let key_agg_ctx = if tweak {
        key_agg_ctx
            .with_unspendable_taproot_tweak()
            .expect("should add unspendable taproot tweak")
    } else {
        key_agg_ctx
    };

    let aggregated_pubkey: PublicKey = key_agg_ctx.aggregated_pubkey();

    let mut nonce_seed = [0u8; 32];
    OsRng.fill_bytes(&mut nonce_seed);

    let sec_nonce = SecNonce::build(nonce_seed)
        .with_seckey(seckey)
        .with_message(msg)
        .with_aggregated_pubkey(aggregated_pubkey)
        .build();

    sec_nonce
}

/// Shuffle a list using a secure random number generator.
/// This is just a trivial wrapper around `rand` functionality.
/// It only exists to simplify imports and allow for easier refactoring.
pub fn permute<T: Clone>(list: &mut [T]) {
    list.shuffle(&mut OsRng);
}

pub fn create_opreturn_metadata(
    magic: [u8; 4],
    operator_idx: u32,
    deposit_idx: u32,
    deposit_txid: &[u8; 32],
) -> ScriptBuf {
    let mut metadata = [0u8; 44];
    metadata[..4].copy_from_slice(&magic);
    // first 4 bytes = operator idx
    metadata[4..8].copy_from_slice(&operator_idx.to_be_bytes());
    // next 4 bytes = deposit idx
    metadata[8..12].copy_from_slice(&deposit_idx.to_be_bytes());
    metadata[12..44].copy_from_slice(deposit_txid);
    bitcoin_bosd::Descriptor::new_op_return(&metadata)
        .unwrap()
        .to_script()
}
