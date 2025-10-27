use argh::FromArgs;
use strata_cli_common::errors::{DisplayableError, DisplayedError};

use crate::taproot::convert_to_xonly_pk_inner;

/// Arguments for converting a public key to X-only format.
///
/// Strips the parity byte from a public key to produce an X-only public key (32 bytes).
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "convert-to-xonly-pk")]
pub struct ConvertToXonlyPkArgs {
    #[argh(option)]
    /// public key in hex format
    pub pubkey: String,
}

pub(crate) fn convert_to_xonly_pk(args: ConvertToXonlyPkArgs) -> Result<(), DisplayedError> {
    let result = convert_to_xonly_pk_inner(args.pubkey).user_error("Invalid public key format")?;
    println!("{}", result);

    Ok(())
}
