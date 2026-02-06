//! OL DA test utilities.

use bitcoin::{
    Amount, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Witness, XOnlyPublicKey,
    absolute,
    key::UntweakedKeypair,
    opcodes::{
        OP_FALSE,
        all::{OP_CHECKSIG, OP_ENDIF, OP_IF},
    },
    script::{self, PushBytesBuf},
    secp256k1::{SECP256K1, schnorr::Signature},
    taproot::{LeafVersion, TaprootBuilder},
    transaction::Version,
};
use ssz::Encode;
use strata_checkpoint_types_ssz::{
    CheckpointPayload, CheckpointSidecar, CheckpointTip, SignedCheckpointPayload,
};
use strata_crypto::{hash, sign_schnorr_sig};
use strata_identifiers::{Buf32, OLBlockCommitment};
use strata_l1_txfmt::{MagicBytes, ParseConfig, TagDataRef};

/// Magic bytes for testing purposes.
pub const TEST_MAGIC_BYTES: MagicBytes = MagicBytes::new(*b"ALPN");

/// Builds reveal script such that it contains opcodes for verifying the internal key as well as the
/// envelope block
fn build_reveal_script(taproot_public_key: &XOnlyPublicKey, payload: &[u8]) -> ScriptBuf {
    let mut script_bytes = script::Builder::new()
        .push_x_only_key(taproot_public_key)
        .push_opcode(OP_CHECKSIG)
        .into_script()
        .into_bytes();
    let script = build_envelope_script(payload);
    script_bytes.extend(script.into_bytes());
    ScriptBuf::from(script_bytes)
}

/// Builds the envelope script for the given payload.
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

/// Creates a test reveal transaction with the provided envelope payload, output tag script, and a
/// secret key.
///
/// This helper is intended for cross-crate tests that need to construct a synthetic
/// SPS-50-tagged reveal transaction without duplicating witness/script boilerplate.
pub fn create_test_reveal_tx(
    envelope_payload: Vec<u8>,
    tag_script: ScriptBuf,
    secret_key: Buf32,
) -> Transaction {
    // Create commit key
    let key_pair = UntweakedKeypair::from_seckey_slice(SECP256K1, secret_key.as_slice()).unwrap();
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
        lock_time: absolute::LockTime::ZERO,
        input: vec![TxIn {
            previous_output: OutPoint::null(),
            script_sig: ScriptBuf::new(),
            sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
            witness,
        }],
        output: vec![TxOut {
            value: Amount::ZERO,
            script_pubkey: tag_script,
        }],
    }
}

/// Creates a signed checkpoint payload.
pub fn make_signed_checkpoint_payload(
    epoch: u32,
    l1_height: u32,
    l2_commitment: OLBlockCommitment,
    state_diff: Vec<u8>,
    secret_key: Buf32,
) -> SignedCheckpointPayload {
    let tip = CheckpointTip::new(epoch, l1_height, l2_commitment);
    let sidecar = CheckpointSidecar::new(state_diff, vec![]).expect("make sidecar");
    let payload = CheckpointPayload::new(tip, sidecar, vec![]).expect("make payload");
    let msg_hash = hash::raw(&payload.as_ssz_bytes());
    let signature = sign_schnorr_sig(&msg_hash, &secret_key);
    SignedCheckpointPayload::new(payload, signature)
}

/// Creates a checkpoint transaction with the given payload, subprotocol, tx type, and secret key.
pub fn make_checkpoint_tx(
    payload: &[u8],
    subprotocol: u8,
    tx_type: u8,
    secret_key: Buf32,
) -> Transaction {
    let tag_data = TagDataRef::new(subprotocol, tx_type, &[]).expect("build tag");
    let tag_script = ParseConfig::new(TEST_MAGIC_BYTES)
        .encode_script_buf(&tag_data)
        .expect("encode tag script");
    create_test_reveal_tx(payload.to_vec(), tag_script, secret_key)
}
