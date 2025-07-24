use argh::FromArgs;
use strata_cli_common::errors::DisplayedError;
use strata_db::traits::Database;

use crate::cli::OutputFormat;

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "get-syncinfo")]
/// Get sync info
pub(crate) struct GetSyncinfoArgs {
    /// output format: "json" or "porcelain"
    #[argh(option, short = 'o', default = "OutputFormat::Porcelain")]
    pub(crate) output_format: OutputFormat,
}

/// Show the latest sync information.
pub(crate) fn get_syncinfo(
    _db: &impl Database,
    _args: GetSyncinfoArgs,
) -> Result<(), DisplayedError> {
    Ok(())
}
