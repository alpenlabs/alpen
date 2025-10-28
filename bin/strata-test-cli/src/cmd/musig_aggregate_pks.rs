use argh::FromArgs;
use strata_cli_common::errors::{DisplayableError, DisplayedError};

use crate::taproot::musig_aggregate_pks_inner;

/// Arguments for aggregating public keys using MuSig2.
///
/// Combines multiple public keys into a single aggregated key using the MuSig2 protocol.
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "musig-aggregate-pks")]
pub struct MusigAggregatePksArgs {
    #[argh(option)]
    /// public keys in JSON array format (33-byte compressed hex strings)
    /// Example: --pubkeys='["foo", "bar"]'
    pub pubkeys: String,
}

pub(crate) fn musig_aggregate_pks(args: MusigAggregatePksArgs) -> Result<(), DisplayedError> {
    let pks: Vec<String> =
        serde_json::from_str(&args.pubkeys).user_error("Invalid pubkeys JSON format")?;

    let result =
        musig_aggregate_pks_inner(pks).internal_error("Failed to aggregate public keys")?;
    println!("{}", result);

    Ok(())
}
