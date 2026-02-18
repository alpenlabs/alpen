//! Test utilities for constructing and manipulating ASM transactions.
//!
//! Provides helpers for creating dummy Bitcoin transactions, building SPS-50
//! tagged reveal transactions with taproot envelope scripts, and parsing or
//! mutating transaction auxiliary data. Intended for use in unit and
//! integration tests across the `asm/txs` crates.

use bitcoin::{
    Amount, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Witness, XOnlyPublicKey,
    absolute::LockTime,
    key::UntweakedKeypair,
    secp256k1::{SECP256K1, schnorr::Signature},
    taproot::{LeafVersion, TaprootBuilder},
    transaction::Version,
};
use rand::{RngCore, rngs::OsRng};
use strata_asm_common::TxInputRef;
use strata_l1_envelope_fmt::builder::EnvelopeScriptBuilder;
use strata_l1_txfmt::{MagicBytes, ParseConfig, TagData};

pub const TEST_MAGIC_BYTES: MagicBytes = MagicBytes::new(*b"ALPN");

/// Creates a dummy Bitcoin transaction with the specified number of inputs and outputs.
///
/// The inputs will have null previous outputs and empty script sigs.
/// The outputs will have zero value and empty script pubkeys.
/// The transaction version is set to 2, and lock time to 0.
pub fn create_dummy_tx(num_inputs: usize, num_outputs: usize) -> Transaction {
    let input = (0..num_inputs)
        .map(|_| TxIn {
            previous_output: OutPoint::null(),
            script_sig: ScriptBuf::new(),
            sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
            witness: Witness::new(),
        })
        .collect();

    let output = (0..num_outputs)
        .map(|_| TxOut {
            value: Amount::ZERO,
            script_pubkey: ScriptBuf::new(),
        })
        .collect();

    Transaction {
        version: Version(2),
        lock_time: LockTime::ZERO,
        input,
        output,
    }
}

/// Creates a stub reveal transaction containing the envelope script.
/// This is a simplified implementation for testing purposes.
pub fn create_reveal_transaction_stub(envelope_payload: Vec<u8>, tag: TagData) -> Transaction {
    // Create commit key
    let mut rand_bytes = [0; 32];
    OsRng.fill_bytes(&mut rand_bytes);
    let key_pair = UntweakedKeypair::from_seckey_slice(SECP256K1, &rand_bytes).unwrap();
    let public_key = XOnlyPublicKey::from_keypair(&key_pair).0;

    // Start creating envelope content
    let reveal_script = EnvelopeScriptBuilder::with_pubkey(&public_key.serialize())
        .unwrap()
        .add_envelope(&envelope_payload)
        .unwrap()
        .build()
        .unwrap();

    let sps50_script = ParseConfig::new(TEST_MAGIC_BYTES)
        .encode_script_buf(&tag.as_ref())
        .unwrap();

    // Create spend info for tapscript
    let taproot_spend_info = TaprootBuilder::new()
        .add_leaf(0, reveal_script.clone())
        .unwrap()
        .finalize(SECP256K1, public_key)
        .expect("Could not build taproot spend info");

    let signature = Signature::from_slice(&[0u8; 64]).unwrap();
    let mut witness = Witness::new();
    witness.push(signature.as_ref());
    witness.push(reveal_script.clone());
    witness.push(
        taproot_spend_info
            .control_block(&(reveal_script, LeafVersion::TapScript))
            .expect("Could not create control block")
            .serialize(),
    );

    Transaction {
        version: Version::TWO,
        lock_time: LockTime::ZERO,
        input: vec![TxIn {
            previous_output: OutPoint::null(),
            script_sig: ScriptBuf::new(),
            sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
            witness,
        }],
        output: vec![TxOut {
            value: Amount::ZERO,
            script_pubkey: sps50_script,
        }],
    }
}

/// Mutates the auxiliary data of an SPS-50 tagged transaction.
pub fn mutate_aux_data(tx: &mut Transaction, new_aux: Vec<u8>) {
    let config = ParseConfig::new(TEST_MAGIC_BYTES);
    let td = config.try_parse_tx(tx).expect("dummy tx must parse");
    let new_td = TagData::new(td.subproto_id(), td.tx_type(), new_aux)
        .expect("tag data construction must succeed");
    let new_scriptbuf = config
        .encode_script_buf(&new_td.as_ref())
        .expect("encoding SPS50 script must succeed");
    tx.output[0].script_pubkey = new_scriptbuf
}

/// Parses a transaction as an SPS-50 tagged transaction.
pub fn parse_sps50_tx(tx: &Transaction) -> TxInputRef<'_> {
    let parser = ParseConfig::new(TEST_MAGIC_BYTES);
    let tag_data = parser.try_parse_tx(tx).expect("Should parse transaction");
    TxInputRef::new(tx, tag_data)
}
