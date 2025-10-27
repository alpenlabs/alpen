use argh::FromArgs;
use strata_cli_common::errors::{DisplayableError, DisplayedError};

use crate::schnorr::sign_schnorr_sig_inner;

/// Arguments for signing a message with a Schnorr signature.
///
/// Creates a Schnorr signature over a message hash using the provided secret key.
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "sign-schnorr-sig")]
pub struct SignSchnorrSigArgs {
    #[argh(option)]
    /// message hash in hex format (32 bytes)
    pub message: String,

    #[argh(option)]
    /// secret key in hex format (32 bytes)
    pub secret_key: String,
}

pub(crate) fn sign_schnorr_sig(args: SignSchnorrSigArgs) -> Result<(), DisplayedError> {
    let (sig, pk) = sign_schnorr_sig_inner(&args.message, &args.secret_key)
        .user_error("Invalid message or secret key")?;
    let output = serde_json::json!({
        "signature": hex::encode(sig),
        "public_key": hex::encode(pk)
    });
    println!("{}", output);

    Ok(())
}
