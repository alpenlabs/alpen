use argh::FromArgs;
use strata_cli_common::errors::{DisplayableError, DisplayedError};

use crate::utils::xonlypk_to_descriptor_inner;

/// Arguments for converting an X-only public key to a BOSD descriptor.
///
/// Generates a Bitcoin Output Script Descriptor (BOSD) from an X-only public key.
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "xonlypk-to-descriptor")]
pub struct XonlypkToDescriptorArgs {
    /// x-only public key in hex-encoded string (32 bytes)
    #[argh(option)]
    pub xonly_pubkey: String,
}

pub(crate) fn xonlypk_to_descriptor(args: XonlypkToDescriptorArgs) -> Result<(), DisplayedError> {
    let result = xonlypk_to_descriptor_inner(&args.xonly_pubkey)
        .user_error("Invalid X-only public key format")?;
    println!("{}", result);

    Ok(())
}
