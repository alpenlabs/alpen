use argh::FromArgs;
use strata_cli_common::errors::{DisplayableError, DisplayedError};

use crate::taproot::convert_to_xonly_pk_inner;

/// Arguments for converting a public key to X-only format.
///
/// Strips the parity byte from a public key to produce an X-only public key (32 bytes).
///
/// # Warning
///
/// This should only be done for even public keys, i.e. the parity byte is `02`
/// and the public key starts with `02...`.
/// The caller is responsible for only using this function on even public keys,
/// since it is implied in Taproot that all X-only public keys are even.
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "convert-to-xonly-pk")]
pub struct ConvertToXonlyPkArgs {
    /// public key in hex-encoded string
    #[argh(option)]
    pub pubkey: String,
}

pub(crate) fn convert_to_xonly_pk(args: ConvertToXonlyPkArgs) -> Result<(), DisplayedError> {
    let result = convert_to_xonly_pk_inner(args.pubkey).user_error("Invalid public key format")?;
    println!("{}", result);

    Ok(())
}
