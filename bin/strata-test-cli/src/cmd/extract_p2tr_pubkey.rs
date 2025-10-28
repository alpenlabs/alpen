use argh::FromArgs;
use strata_cli_common::errors::{DisplayableError, DisplayedError};

use crate::taproot::extract_p2tr_pubkey_inner;

/// Arguments for extracting the P2TR public key from a taproot address.
///
/// Parses a taproot (P2TR) address and extracts the embedded X-only public key.
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "extract-p2tr-pubkey")]
pub struct ExtractP2trPubkeyArgs {
    /// taproot address
    #[argh(option)]
    pub address: String,
}

pub(crate) fn extract_p2tr_pubkey(args: ExtractP2trPubkeyArgs) -> Result<(), DisplayedError> {
    let result = extract_p2tr_pubkey_inner(args.address).user_error("Invalid taproot address")?;
    println!("{}", result);

    Ok(())
}
