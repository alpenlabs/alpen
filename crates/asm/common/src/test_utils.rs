//! Test utilities for the asm-common crate.

use bitcoin::{
    Amount, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Witness, XOnlyPublicKey,
    absolute::LockTime,
    blockdata::script,
    key::UntweakedKeypair,
    opcodes::{
        OP_FALSE,
        all::{OP_CHECKSIG, OP_ENDIF, OP_IF},
    },
    script::PushBytesBuf,
    secp256k1::{SECP256K1, schnorr::Signature},
    taproot::{LeafVersion, TaprootBuilder},
    transaction::Version,
};
use rand::{RngCore, rngs::OsRng};

/// Creates a stub reveal transaction containing the envelope script.
/// This is a simplified implementation for testing purposes.
pub fn create_reveal_transaction_stub(
    envelope_payload: Vec<u8>,
    sps50_tagged_payload: Vec<u8>,
) -> Transaction {
    // Create commit key
    let mut rand_bytes = [0; 32];
    OsRng.fill_bytes(&mut rand_bytes);
    let key_pair = UntweakedKeypair::from_seckey_slice(SECP256K1, &rand_bytes).unwrap();
    let public_key = XOnlyPublicKey::from_keypair(&key_pair).0;

    // Start creating envelope content
    let reveal_script = build_reveal_script(&public_key, &envelope_payload);

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
            script_pubkey: ScriptBuf::new_op_return(
                PushBytesBuf::try_from(sps50_tagged_payload).unwrap(),
            ),
        }],
    }
}

/// Builds reveal script such that it contains opcodes for verifying the internal key as well as the
/// envelope block
pub fn build_reveal_script(taproot_public_key: &XOnlyPublicKey, payload: &[u8]) -> ScriptBuf {
    let mut script_bytes = script::Builder::new()
        .push_x_only_key(taproot_public_key)
        .push_opcode(OP_CHECKSIG)
        .into_script()
        .into_bytes();
    let script = build_envelope_script(payload);
    script_bytes.extend(script.into_bytes());
    ScriptBuf::from(script_bytes)
}

fn build_envelope_script(payload: &[u8]) -> ScriptBuf {
    let mut builder = script::Builder::new()
        .push_opcode(OP_FALSE)
        .push_opcode(OP_IF);

    // Insert actual data
    for chunk in payload.chunks(520) {
        builder = builder.push_slice(PushBytesBuf::try_from(chunk.to_vec()).unwrap());
    }
    builder = builder.push_opcode(OP_ENDIF);
    builder.into_script()
}
