use argh::FromArgs;
use strata_cli_common::errors::{DisplayableError, DisplayedError};

use crate::taproot::get_address_inner;

/// Arguments for retrieving a taproot address.
///
/// Generates a taproot address at the specified derivation index.
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "get-address")]
pub struct GetAddressArgs {
    #[argh(option)]
    /// address index
    pub index: u32,
}

pub(crate) fn get_address(args: GetAddressArgs) -> Result<(), DisplayedError> {
    let address =
        get_address_inner(args.index).internal_error("Failed to generate taproot address")?;
    println!("{}", address);

    Ok(())
}
