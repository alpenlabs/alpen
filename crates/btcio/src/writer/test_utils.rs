use std::sync::Arc;

use bitcoin::{
    hashes::Hash,
    key::TapTweak,
    secp256k1::{schnorr::Signature, Keypair, Message, XOnlyPublicKey, SECP256K1},
    sighash::{Prevouts, SighashCache, TapSighashType},
    taproot::TapNodeHash,
    Transaction, TxOut,
};
use strata_crypto::keys::constants::UNSPENDABLE_PUBLIC_KEY;
use strata_db_store_sled::{
    test_utils::{get_test_sled_backend, get_test_sled_config, get_test_sled_db},
    SledBackend,
};
use strata_db_types::backend::DatabaseBackend;
use strata_storage::{
    ops::{chunked_envelope::ChunkedEnvelopeOps, writer::EnvelopeDataOps},
    test_runtime_handle, BroadcastDbOps,
};

use crate::{
    broadcaster::L1BroadcastHandle,
    writer::replacement::build::{
        compute_taproot_script_spend_sighash, extract_reveal_witness, reveal_script_pubkey,
    },
};

/// Checks a valid reveal and rejects a key-path bypass using the reveal signer's key.
pub(crate) fn assert_reveal_requires_script_path(
    reveal: &Transaction,
    commit_output: &TxOut,
    signer: &Keypair,
) {
    let (script, control_block) = extract_reveal_witness(reveal).unwrap();
    let output_key =
        XOnlyPublicKey::from_slice(&commit_output.script_pubkey.as_bytes()[2..]).unwrap();
    let signer_pubkey = signer.x_only_public_key().0;
    assert_eq!(reveal_script_pubkey(&script).unwrap(), signer_pubkey);
    assert!(control_block.verify_taproot_commitment(SECP256K1, output_key, &script));

    let script_sighash =
        compute_taproot_script_spend_sighash(reveal, commit_output, &script).unwrap();
    let signature = Signature::from_slice(reveal.input[0].witness.nth(0).unwrap()).unwrap();
    let message = Message::from_digest(script_sighash);
    SECP256K1
        .verify_schnorr(&signature, &message, &signer_pubkey)
        .unwrap();

    // Both builders use one leaf. Try spending with only a tweaked-key signature,
    // omitting the script and its payload entirely.
    assert!(control_block.merkle_branch.is_empty());
    let merkle_root = TapNodeHash::from_script(&script, control_block.leaf_version);
    let tweaked_signer = signer.tap_tweak(SECP256K1, Some(merkle_root)).to_keypair();
    let key_sighash = SighashCache::new(reveal)
        .taproot_key_spend_signature_hash(
            0,
            &Prevouts::All(&[commit_output]),
            TapSighashType::Default,
        )
        .unwrap();
    let message = Message::from_digest(key_sighash.to_byte_array());
    let signature = SECP256K1.sign_schnorr(&message, &tweaked_signer);
    assert!(
        SECP256K1
            .verify_schnorr(&signature, &message, &output_key)
            .is_err(),
        "reveal signer can bypass the payload script through a key-path spend"
    );
    assert_eq!(control_block.internal_key, *UNSPENDABLE_PUBLIC_KEY);
}

/// Returns [`Arc`] of [`EnvelopeDataOps`] for testing
pub(crate) fn get_envelope_ops() -> Arc<EnvelopeDataOps> {
    let db = get_test_sled_backend().writer_db();
    let ops = EnvelopeDataOps::new(test_runtime_handle(), db);
    Arc::new(ops)
}

/// Returns [`Arc`] of [`ChunkedEnvelopeOps`] for testing.
pub(crate) fn get_chunked_envelope_ops() -> Arc<ChunkedEnvelopeOps> {
    let db = get_test_sled_backend().chunked_envelope_db();
    let ops = ChunkedEnvelopeOps::new(test_runtime_handle(), db);
    Arc::new(ops)
}

/// Returns [`Arc`] of [`L1BroadcastHandle`] for testing
pub(crate) fn get_broadcast_handle() -> Arc<L1BroadcastHandle> {
    let sdb = get_test_sled_db();
    let sconf = get_test_sled_config();
    let backend = SledBackend::new(sdb.into(), sconf).unwrap();
    let db = backend.broadcast_db();
    let ops = BroadcastDbOps::new(test_runtime_handle(), db);
    let (handle, _) = L1BroadcastHandle::new_for_test(Arc::new(ops));
    Arc::new(handle)
}
