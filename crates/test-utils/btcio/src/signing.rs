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

/// Helper function to compute taproot sighash for MuSig2 signing.
fn compute_taproot_sighash(
    tx: &Transaction,
    prevouts: &[TxOut],
    input_index: usize,
) -> anyhow::Result<[u8; 32]> {
    let prevouts_ref = Prevouts::All(prevouts);
    let mut sighash_cache = SighashCache::new(tx);
    let sighash = sighash_cache.taproot_key_spend_signature_hash(
        input_index,
        &prevouts_ref,
        TapSighashType::Default,
    )?;
    Ok(*sighash.as_ref())
}

/// Sign a transaction with MuSig2 aggregated signature for taproot key path spend.
///
/// This function applies the taproot tweak to the aggregated public key, which is required
/// for key path spends. The signature will verify against the tweaked output key.
///
/// # Returns
/// The aggregated Schnorr signature
pub fn sign_musig2_keypath(
    tx: &Transaction,
    secret_keys: &[EvenSecretKey],
    prevouts: &[TxOut],
    input_index: usize,
) -> anyhow::Result<Signature> {
    let sighash_bytes = compute_taproot_sighash(tx, prevouts, input_index)?;

    // Taproot key-path spend without a script tree uses the standard tweak with an empty merkle
    // root. Musig2 helper applies that tweak when using the TaprootKeySpend variant.
    let compact_sig =
        create_musig2_signature(secret_keys, &sighash_bytes, Musig2Tweak::TaprootKeySpend);

    // Convert CompactSignature to bitcoin::secp256k1::schnorr::Signature
    let sig = Signature::from_slice(&compact_sig.serialize())?;

    Ok(sig)
}

/// Sign a transaction with MuSig2 aggregated signature for taproot script path spend.
///
/// This function does NOT apply the taproot tweak, as script path spends use the control
/// block to prove script execution. The signature must verify against the raw pubkey in
/// the script, not the tweaked output key.
///
/// For script path spends, we need to compute the script-specific sighash that includes
/// the leaf hash of the script being executed.
///
/// # Returns
/// The aggregated Schnorr signature
pub fn sign_musig2_scriptpath(
    tx: &Transaction,
    secret_keys: &[EvenSecretKey],
    prevouts: &[TxOut],
    input_index: usize,
    script: &bitcoin::ScriptBuf,
    leaf_version: bitcoin::taproot::LeafVersion,
) -> anyhow::Result<Signature> {
    // For script path spends, we need to use taproot_script_spend_signature_hash
    let prevouts_ref = Prevouts::All(prevouts);
    let mut sighash_cache = SighashCache::new(tx);
    let sighash = sighash_cache.taproot_script_spend_signature_hash(
        input_index,
        &prevouts_ref,
        bitcoin::taproot::TapLeafHash::from_script(script, leaf_version),
        TapSighashType::Default,
    )?;
    let sighash_bytes: [u8; 32] = *sighash.as_ref();

    // For script path spends, no tweak is applied. The signature verifies against the
    // pubkey embedded in the script itself.
    let compact_sig = create_musig2_signature(secret_keys, &sighash_bytes, Musig2Tweak::None);

    // Convert CompactSignature to bitcoin::secp256k1::schnorr::Signature
    let sig = Signature::from_slice(&compact_sig.serialize())?;

    Ok(sig)
}
