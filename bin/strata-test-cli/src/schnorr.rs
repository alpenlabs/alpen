use std::str::FromStr;

use anyhow::Context;
use secp256k1::{Keypair, SecretKey, SECP256K1};
use strata_crypto::sign_schnorr_sig as sign_schnorr_crypto;
use strata_primitives::buf::Buf32;

/// Signs a message using the Schnorr signature scheme.
///
/// Generates a Schnorr signature for the given message using the provided secret key.
/// Returns the serialized signature and the corresponding public key.
///
/// # Arguments
/// * `message` - A string representing the message to sign, encoded in hexadecimal format.
/// * `secret_key` - A string representing the secret key, encoded in hexadecimal format.
///
/// # Returns
/// * The Schnorr signature (64 bytes)
/// * The public key (32 bytes)
pub(crate) fn sign_schnorr_inner(
    message: &str,
    secret_key: &str,
) -> anyhow::Result<(Vec<u8>, Vec<u8>)> {
    let message = Buf32::from_str(message).context("invalid message hash")?;
    let sk = Buf32::from_str(secret_key).context("invalid secret key")?;

    let sig = sign_schnorr_crypto(&message, &sk);

    // get the public key
    let sk = SecretKey::from_str(secret_key).context("invalid secret key")?;
    let keypair = Keypair::from_secret_key(SECP256K1, &sk);
    let x_only_pubkey = keypair.x_only_public_key();

    Ok((
        sig.as_slice().to_vec(),              // Signature (64 bytes)
        x_only_pubkey.0.serialize().to_vec(), // Public key (32 bytes)
    ))
}
