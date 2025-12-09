use bitcoin::{
    TapSighashType, Transaction, TxOut, XOnlyPublicKey,
    secp256k1::{Keypair, Message, Secp256k1, schnorr::Signature},
    sighash::{Prevouts, SighashCache},
    taproot::TapTweakHash,
};
use strata_crypto::{
    EvenSecretKey,
    test_utils::schnorr::{Musig2Tweak, create_musig2_signature},
};

/// Sign a transaction with a taproot key-spend signature.
pub fn sign_taproot_transaction(
    tx: &Transaction,
    keypair: &Keypair,
    internal_key: &XOnlyPublicKey,
    prev_output: &TxOut,
    input_index: usize,
) -> anyhow::Result<Signature> {
    let secp = Secp256k1::new();

    // Apply BIP341 taproot tweak
    let tweak = TapTweakHash::from_key_and_tweak(*internal_key, None);
    let tweaked_keypair = keypair.add_xonly_tweak(&secp, &tweak.to_scalar())?;

    let prevouts = vec![prev_output.clone()];
    let prevouts_ref = Prevouts::All(&prevouts);
    let mut sighash_cache = SighashCache::new(tx);
    let sighash = sighash_cache.taproot_key_spend_signature_hash(
        input_index,
        &prevouts_ref,
        TapSighashType::Default,
    )?;

    let msg = Message::from_digest_slice(sighash.as_ref())?;
    let signature = secp.sign_schnorr_no_aux_rand(&msg, &tweaked_keypair);

    Ok(signature)
}

/// Sign a transaction with MuSig2 aggregated signature.
///
/// # Returns
/// The aggregated Schnorr signature
pub fn sign_musig2_transaction(
    tx: &Transaction,
    secret_keys: &[EvenSecretKey],
    _internal_key: &XOnlyPublicKey,
    prevouts: &[TxOut],
    input_index: usize,
) -> anyhow::Result<Signature> {
    // Calculate sighash
    let prevouts_ref = Prevouts::All(prevouts);
    let mut sighash_cache = SighashCache::new(tx);
    let sighash = sighash_cache.taproot_key_spend_signature_hash(
        input_index,
        &prevouts_ref,
        TapSighashType::Default,
    )?;

    // Taproot key-path spend without a script tree uses the standard tweak with an empty merkle
    // root. Musig2 helper applies that tweak when using the TaprootKeySpend variant.
    let sighash_bytes: &[u8; 32] = sighash.as_ref();
    let compact_sig =
        create_musig2_signature(secret_keys, sighash_bytes, Musig2Tweak::TaprootKeySpend);

    // Convert CompactSignature to bitcoin::secp256k1::schnorr::Signature
    let sig = Signature::from_slice(&compact_sig.serialize())?;

    Ok(sig)
}
