//! MuSig2 signer for bridge transactions
//!
//! Adapted from mock-bridge implementation for use in python-utils.
//! Provides multi-signature capabilities for operator keys.

use bdk_wallet::bitcoin::{
    hashes::Hash,
    key::Keypair,
    secp256k1::{All, Secp256k1, XOnlyPublicKey},
    sighash::{Prevouts, SighashCache, TapSighashType},
    taproot::Signature,
    Psbt, TxOut,
};
use musig2::{
    secp256k1::schnorr, CompactSignature, FirstRound, KeyAggContext, PartialSignature, PubNonce,
    SecNonceSpices, SecondRound,
};
use rand::{thread_rng, RngCore};
use secp256k1::Parity;

// Removed complex type dependencies - working with basic bitcoin types directly
use crate::error::Error;

/// MuSig2 signer for bridge transactions
pub(crate) struct MusigSigner {
    #[allow(dead_code)]
    secp: Secp256k1<All>,
}

impl MusigSigner {
    /// Create a new MuSig signer instance
    pub(crate) fn new() -> Self {
        Self {
            secp: Secp256k1::new(),
        }
    }

    /// Creates an aggregated signature for the deposit transaction PSBT using MuSig2
    ///
    /// # Arguments
    /// * `psbt` - The PSBT to sign
    /// * `prevouts` - Previous outputs for the transaction
    /// * `tweak` - Optional taproot tweak hash
    /// * `keypairs` - Vector of operator keypairs for multi-signature
    /// * `input_index` - Index of the input to sign (usually 0 for deposit transactions)
    ///
    /// # Returns
    /// * `Result<Signature, Error>` - The aggregated taproot signature
    pub(crate) fn sign_deposit_psbt(
        &self,
        psbt: &Psbt,
        prevouts: &[TxOut],
        tweak: Option<bdk_wallet::bitcoin::TapNodeHash>,
        keypairs: Vec<Keypair>,
        input_index: usize,
    ) -> Result<Signature, Error> {
        if keypairs.is_empty() {
            return Err(Error::Musig("No operator keys provided".to_string()));
        }

        let mut full_pubkeys = Vec::new();
        for kp in &keypairs {
            // Convert to x-only and then to full public key with even parity
            let (xonly, _parity) = XOnlyPublicKey::from_keypair(kp);
            // Always use even parity for the full public key to ensure consistency
            let even_full = xonly.public_key(Parity::Even);
            full_pubkeys.push(even_full);
        }

        // Create key aggregation context with full public keys
        let mut ctx = KeyAggContext::new(full_pubkeys.iter().cloned())
            .map_err(|e| Error::Musig(format!("Key aggregation failed: {}", e)))?;

        // Apply taproot tweak based on provided tweak
        if let Some(tweak_hash) = tweak {
            ctx = ctx
                .with_taproot_tweak(tweak_hash.as_ref())
                .map_err(|e| Error::Musig(format!("Taproot tweak failed: {}", e)))?;
        } else {
            // Use unspendable taproot tweak if no specific tweak provided
            ctx = ctx
                .with_unspendable_taproot_tweak()
                .map_err(|e| Error::Musig(format!("Unspendable taproot tweak failed: {}", e)))?;
        }
        let ct: XOnlyPublicKey = ctx.aggregated_pubkey();
        println!("{}", ct);

        // Create sighash for the transaction
        let sighash = self.create_sighash(&psbt.unsigned_tx, prevouts, input_index)?;

        // First round: Generate nonces
        let mut first_rounds: Vec<FirstRound> = Vec::new();
        let mut pub_nonces: Vec<PubNonce> = Vec::new();

        // Create first rounds with proper signer indices
        for (signer_index, keypair) in keypairs.iter().enumerate() {
            let spices = SecNonceSpices::new()
                .with_seckey(keypair.secret_key())
                .with_message(sighash.as_byte_array());

            // Generate a proper nonce seed
            let mut nonce_seed = [0u8; 32];
            thread_rng().fill_bytes(&mut nonce_seed);

            let first_round: FirstRound =
                FirstRound::new(ctx.clone(), nonce_seed, signer_index, spices)
                    .map_err(|e| Error::Musig(format!("First round creation failed: {}", e)))?;

            let pub_nonce = first_round.our_public_nonce();
            pub_nonces.push(pub_nonce);
            first_rounds.push(first_round);
        }

        // Exchange public nonces between all signers
        for (i, first_round) in first_rounds.iter_mut().enumerate() {
            for (j, pub_nonce) in pub_nonces.iter().enumerate() {
                if i != j {
                    first_round
                        .receive_nonce(j, pub_nonce.clone())
                        .map_err(|e| Error::Musig(format!("Nonce exchange failed: {}", e)))?;
                }
            }
        }

        // Second round: Generate partial signatures
        let mut second_rounds: Vec<SecondRound<&[u8; 32]>> = Vec::new();
        let mut partial_sigs: Vec<PartialSignature> = Vec::new();

        for (i, first_round) in first_rounds.into_iter().enumerate() {
            if !first_round.is_complete() {
                return Err(Error::Musig("First round not complete".to_string()));
            }

            // Use the same keypair index as in the first round
            let keypair = &keypairs[i];

            let second_round = first_round
                .finalize(keypair.secret_key(), sighash.as_byte_array())
                .map_err(|e| Error::Musig(format!("Second round finalization failed: {}", e)))?;

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
                        .map_err(|e| Error::Musig(format!("Signature exchange failed: {}", e)))?;
                }
            }
        }

        // Finalize aggregated signature using the first signer's second round
        let aggregated_sig: CompactSignature = second_rounds
            .into_iter()
            .next()
            .ok_or_else(|| Error::Musig("No second rounds available".to_string()))?
            .finalize()
            .map_err(|e| Error::Musig(format!("Signature aggregation failed: {}", e)))?;

        // Convert to Bitcoin taproot signature
        let taproot_sig = Signature {
            signature: schnorr::Signature::from_slice(&aggregated_sig.serialize())
                .map_err(|e| Error::Musig(format!("Invalid signature format: {}", e)))?,
            sighash_type: TapSighashType::Default,
        };

        Ok(taproot_sig)
    }

    /// Creates the sighash for the transaction input
    fn create_sighash(
        &self,
        tx: &bdk_wallet::bitcoin::Transaction,
        prevouts: &[TxOut],
        input_index: usize,
    ) -> Result<bdk_wallet::bitcoin::TapSighash, Error> {
        let prevouts = Prevouts::All(prevouts);
        let mut sighash_cache = SighashCache::new(tx);

        sighash_cache
            .taproot_key_spend_signature_hash(input_index, &prevouts, TapSighashType::Default)
            .map_err(|e| Error::Musig(format!("Sighash creation failed: {}", e)))
    }
}

impl Default for MusigSigner {
    fn default() -> Self {
        Self::new()
    }
}

