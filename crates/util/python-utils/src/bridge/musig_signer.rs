//! MuSig2 signer for bridge transactions
//!
//! Adapted from mock-bridge implementation for use in python-utils.
//! Provides multi-signature capabilities for operator keys.

use bdk_wallet::bitcoin::{
    hashes::Hash,
    sighash::{Prevouts, SighashCache, TapSighashType},
    taproot::Signature,
    Psbt, TapNodeHash, TapSighash, Transaction, TxOut,
};
use musig2::{
    secp256k1::{schnorr::Signature as SchnorrSignature, Parity, SECP256K1},
    CompactSignature, FirstRound, KeyAggContext, PartialSignature, SecNonceSpices, SecondRound,
};
use rand::{rngs::OsRng, RngCore as _};
use strata_crypto::EvenSecretKey;

// Removed complex type dependencies - working with basic bitcoin types directly
use crate::error::Error;

/// MuSig2 signer for bridge transactions
pub(crate) struct MusigSigner;

impl MusigSigner {
    /// Creates an aggregated signature for the deposit transaction PSBT using MuSig2
    ///
    /// # Arguments
    ///
    /// * `psbt` - The PSBT to sign
    /// * `prevouts` - Previous outputs for the transaction
    /// * `tweak` - Optional taproot tweak hash
    /// * `signers` - Vector of signers for multi-signature
    /// * `input_index` - Index of the input to sign (usually 0 for deposit transactions)
    ///
    /// # Returns
    /// * `Result<Signature, Error>` - The aggregated taproot signature
    pub(crate) fn sign_deposit_psbt(
        &self,
        psbt: &Psbt,
        prevouts: &[TxOut],
        tweak: Option<TapNodeHash>,
        signers: Vec<EvenSecretKey>,
        input_index: usize,
    ) -> Result<Signature, Error> {
        if signers.is_empty() {
            return Err(Error::Musig("No signers provided".to_string()));
        }

        let pubkeys = signers
            .iter()
            .map(|kp| kp.x_only_public_key(SECP256K1).0.public_key(Parity::Even))
            .collect::<Vec<_>>();

        // Create key aggregation context with full public keys
        let mut ctx = KeyAggContext::new(pubkeys)
            .map_err(|e| Error::Musig(format!("Key aggregation failed: {e}")))?;

        // Apply taproot tweak based on provided tweak
        if let Some(tweak_hash) = tweak {
            ctx = ctx
                .with_taproot_tweak(tweak_hash.as_ref())
                .map_err(|e| Error::Musig(format!("Taproot tweak failed: {e}")))?;
        } else {
            // Use unspendable taproot tweak if no specific tweak provided
            ctx = ctx
                .with_unspendable_taproot_tweak()
                .map_err(|e| Error::Musig(format!("Unspendable taproot tweak failed: {e}")))?;
        }

        // Create sighash for the transaction
        let sighash = self.create_sighash(&psbt.unsigned_tx, prevouts, input_index)?;

        // First round: generate nonces and collect pub_nonces
        let (mut first_rounds, pub_nonces): (Vec<_>, Vec<_>) = signers
            .iter()
            .enumerate()
            .map(|(signer_index, signer)| {
                let spices = SecNonceSpices::new()
                    .with_seckey(*signer.as_ref())
                    .with_message(sighash.as_byte_array());

                // Generate a proper nonce seed
                let mut nonce_seed = [0u8; 32];
                OsRng.fill_bytes(&mut nonce_seed);

                let first_round = FirstRound::new(ctx.clone(), nonce_seed, signer_index, spices)
                    .map_err(|e| Error::Musig(format!("First round creation failed: {e}")))?;
                let pub_nonce = first_round.our_public_nonce();

                Ok((first_round, pub_nonce))
            })
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .unzip();

        // Exchange public nonces between all signers
        for (i, first_round) in first_rounds.iter_mut().enumerate() {
            pub_nonces
                .iter()
                .enumerate()
                .filter(|(j, _)| *j != i)
                .try_for_each(|(j, pub_nonce)| {
                    first_round
                        .receive_nonce(j, pub_nonce.clone())
                        .map_err(|e| Error::Musig(format!("Nonce exchange failed: {e}")))
                })?;
        }

        // Second round: Generate partial signatures
        let mut second_rounds: Vec<SecondRound<&[u8; 32]>> = Vec::new();
        let mut partial_sigs: Vec<PartialSignature> = Vec::new();

        for (i, first_round) in first_rounds.into_iter().enumerate() {
            if !first_round.is_complete() {
                return Err(Error::Musig("First round not complete".to_string()));
            }

            // Use the same keypair index as in the first round
            let signer = &signers[i];

            let second_round = first_round
                .finalize(*signer.as_ref(), sighash.as_byte_array())
                .map_err(|e| Error::Musig(format!("Second round finalization failed: {e}")))?;

            let partial_sig = second_round.our_signature();
            partial_sigs.push(partial_sig);
            second_rounds.push(second_round);
        }

        // Exchange partial signatures
        for (i, second_round) in second_rounds.iter_mut().enumerate() {
            for (j, partial_sig) in partial_sigs.iter().enumerate() {
                if i != j {
                    second_round
                        .receive_signature(j, *partial_sig)
                        .map_err(|e| Error::Musig(format!("Signature exchange failed: {e}")))?;
                }
            }
        }

        // Finalize aggregated signature using the first signer's second round
        let aggregated_sig: CompactSignature = second_rounds
            .into_iter()
            .next()
            .ok_or_else(|| Error::Musig("No second rounds available".to_string()))?
            .finalize()
            .map_err(|e| Error::Musig(format!("Signature aggregation failed: {e}")))?;

        // Convert to Bitcoin taproot signature
        let taproot_sig = Signature {
            signature: SchnorrSignature::from_slice(&aggregated_sig.serialize())
                .map_err(|e| Error::Musig(format!("Invalid signature format: {e}")))?,
            sighash_type: TapSighashType::Default,
        };

        Ok(taproot_sig)
    }

    /// Creates the sighash for the transaction input
    fn create_sighash(
        &self,
        tx: &Transaction,
        prevouts: &[TxOut],
        input_index: usize,
    ) -> Result<TapSighash, Error> {
        let prevouts = Prevouts::All(prevouts);
        let mut sighash_cache = SighashCache::new(tx);

        sighash_cache
            .taproot_key_spend_signature_hash(input_index, &prevouts, TapSighashType::Default)
            .map_err(|e| Error::Musig(format!("Sighash creation failed: {e}")))
    }
}
