use bitcoin::{
    secp256k1::{PublicKey, Secp256k1, SecretKey},
    XOnlyPublicKey,
};
use musig2::{FirstRound, KeyAggContext, SecNonceSpices};
use rand::{rngs::OsRng, RngCore};
use strata_primitives::{buf::Buf32, crypto::EvenSecretKey};

use crate::multisig::aggregate_schnorr_keys;

/// Creates a MuSig2 signature from multiple operators.
///
/// This function simulates the MuSig2 signing process where multiple operators
/// coordinate to create a single aggregated signature.
///
/// # Arguments
/// - `operators_privkeys`: Private keys of all operators participating in signing
/// - `message`: The message to be signed (typically a sighash)
/// - `tweak`: Optional tweak for taproot spending (merkle root)
///
/// # Returns
/// The aggregated MuSig2 signature
pub fn create_musig2_signature(
    signer_secretkeys: &[SecretKey],
    message: &[u8; 32],
    tweak: Option<[u8; 32]>,
) -> musig2::CompactSignature {
    let secp = Secp256k1::new();

    // Adjust both public keys and private keys for even parity
    let adjusted_keys: Vec<(PublicKey, SecretKey)> = signer_secretkeys
        .iter()
        .map(|sk| {
            let even_sk = EvenSecretKey::from(*sk);
            let pk = PublicKey::from_secret_key(&secp, &even_sk);
            (pk, *even_sk.as_ref())
        })
        .collect();

    // Create KeyAggContext with even parity public keys
    let mut key_agg_ctx =
        KeyAggContext::new(adjusted_keys.iter().map(|(pk, _)| *pk).collect::<Vec<_>>())
            .expect("failed to create KeyAggContext");

    // Apply tweak if provided (for taproot spending)
    if let Some(tweak) = tweak {
        key_agg_ctx = key_agg_ctx
            .with_taproot_tweak(&tweak)
            .expect("Failed to apply taproot tweak to key aggregation context");
    }

    let mut first_rounds = Vec::new();
    let mut public_nonces = Vec::new();

    // Phase 1: Generate nonces for each signer
    for (signer_index, (_, adjusted_privkey)) in adjusted_keys.iter().enumerate() {
        // Generate secure random nonce seed for each signer
        let mut nonce_seed = [0u8; 32];
        OsRng.fill_bytes(&mut nonce_seed);

        let first_round = FirstRound::new(
            key_agg_ctx.clone(),
            nonce_seed,
            signer_index,
            SecNonceSpices::new()
                .with_seckey(*adjusted_privkey)
                .with_message(message),
        )
        .expect("Failed to create FirstRound");

        public_nonces.push(first_round.our_public_nonce());
        first_rounds.push(first_round);
    }

    // Phase 2: Exchange nonces and create partial signatures
    let mut second_rounds = Vec::new();
    for (signer_index, mut first_round) in first_rounds.into_iter().enumerate() {
        // Each signer receives nonces from all other signers
        for (other_index, public_nonce) in public_nonces.iter().enumerate() {
            if other_index != signer_index {
                first_round
                    .receive_nonce(other_index, public_nonce.clone())
                    .expect("Failed to receive nonce");
            }
        }

        // Finalize first round to create second round
        let second_round = first_round
            .finalize(adjusted_keys[signer_index].1, *message)
            .expect("Failed to finalize first round");

        second_rounds.push(second_round);
    }

    // Phase 3: Exchange partial signatures
    let partial_signatures: Vec<musig2::PartialSignature> = second_rounds
        .iter()
        .map(|round| round.our_signature::<musig2::PartialSignature>())
        .collect();

    // Use the first signer to finalize (any signer can do this)
    if let Some((signer_index, mut second_round)) = second_rounds.into_iter().enumerate().next() {
        for (other_index, partial_sig) in partial_signatures.iter().enumerate() {
            if other_index != signer_index {
                second_round
                    .receive_signature(other_index, *partial_sig)
                    .expect("Failed to receive partial signature");
            }
        }

        // Finalize to get the aggregated signature
        return second_round
            .finalize()
            .expect("Failed to finalize MuSig2 signature");
    }

    panic!("No signers available to finalize signature");
}

pub fn create_agg_pubkey_from_privkeys(operators_privkeys: &[SecretKey]) -> XOnlyPublicKey {
    let pubkeys: Vec<_> = operators_privkeys
        .iter()
        .map(|sk| PublicKey::from_secret_key(&Secp256k1::new(), sk))
        .map(|pk| pk.x_only_public_key().0)
        .map(|xpk| Buf32::from(xpk.serialize()))
        .collect();
    aggregate_schnorr_keys(pubkeys.iter()).expect("generation of aggregated public key failed")
}

#[cfg(test)]
mod tests {
    use bitcoin::{
        hashes::Hash,
        key::TapTweak,
        secp256k1::{Secp256k1, SecretKey},
        TapNodeHash,
    };
    use rand::rngs::OsRng;

    use super::*;

    #[test]
    fn test_musig2_signature_validation() {
        let secp = Secp256k1::new();

        // Test message to sign - use random message
        let mut message = [0u8; 32];
        OsRng.fill_bytes(&mut message);

        // Test with tweak (taproot spending) - use random tweak
        let mut tweak = [0u8; 32];
        OsRng.fill_bytes(&mut tweak);

        // Generate test private keys for 3 operators
        let operator_privkeys: Vec<SecretKey> =
            (0..3).map(|_| SecretKey::new(&mut OsRng)).collect();

        // Test without tweak
        let signature_no_tweak = create_musig2_signature(&operator_privkeys, &message, None);

        let signature_with_tweak =
            create_musig2_signature(&operator_privkeys, &message, Some(tweak));

        // Signatures should be different due to different tweaks
        assert_ne!(
            signature_no_tweak.serialize(),
            signature_with_tweak.serialize()
        );

        let agg_pubkey_no_tweak = create_agg_pubkey_from_privkeys(&operator_privkeys);
        let agg_pubkey_with_tweak = agg_pubkey_no_tweak
            .tap_tweak(&secp, Some(TapNodeHash::from_byte_array(tweak)))
            .0
            .to_x_only_public_key();

        // Verify signature without tweak
        let verification_result = secp.verify_schnorr(
            &bitcoin::secp256k1::schnorr::Signature::from_slice(&signature_no_tweak.serialize())
                .expect("Valid signature"),
            &bitcoin::secp256k1::Message::from_digest(message),
            &agg_pubkey_no_tweak,
        );
        assert!(verification_result.is_ok());

        // Verify signature with tweak
        let tweaked_verification_result = secp.verify_schnorr(
            &bitcoin::secp256k1::schnorr::Signature::from_slice(&signature_with_tweak.serialize())
                .expect("Valid signature"),
            &bitcoin::secp256k1::Message::from_digest(message),
            &agg_pubkey_with_tweak,
        );
        assert!(tweaked_verification_result.is_ok());
    }
}
