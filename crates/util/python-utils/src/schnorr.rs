use std::str::FromStr;

use pyo3::{prelude::*, pyfunction, types::PyBytes};
use secp256k1::{Keypair, SecretKey, SECP256K1};
use strata_crypto::sign_schnorr_sig as sign_schnorr_sig_inner;
use strata_primitives::buf::Buf32;

/// Signs a message using the Schnorr signature scheme.
///
/// Generates a Schnorr signature for the given message using the provided secret key.
/// Returns the serialized signature and the corresponding public key.
///
/// # Arguments
/// * `py` - Python interpreter provided by PyO3 for ensuring thread safety
/// * `message` - A string representing the message to sign, encoded in hexadecimal format.
/// * `secret_key` - A string representing the secret key, encoded in hexadecimal format.
///
/// # Returns
/// * The Schnorr signature
/// * The public key
#[pyfunction]
pub(crate) fn sign_schnorr_sig(
    py: Python<'_>,
    message: &str,
    secret_key: &str,
) -> PyResult<(Py<PyBytes>, Py<PyBytes>)> {
    let message = Buf32::from_str(message).expect("invalid message hash");
    let sk = Buf32::from_str(secret_key).expect("invalid secret key");

    let sig = sign_schnorr_sig_inner(&message, &sk);

    // get the public key
    let sk = SecretKey::from_str(secret_key).expect("Invalid secret key");
    let keypair = Keypair::from_secret_key(SECP256K1, &sk);
    let x_only_pubkey = keypair.x_only_public_key();

    Ok((
        PyBytes::new(py, sig.as_slice()).into(), // Signature (64 bytes)
        PyBytes::new(py, &x_only_pubkey.0.serialize()).into(), // Public key (32 bytes)
    ))
}

