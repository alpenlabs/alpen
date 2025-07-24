use argh::FromArgs;
use strata_db::traits::DatabaseBackend;

use crate::cli::OutputFormat;

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "get-client-state-update")]
/// Get client state update
pub(crate) struct GetClientStateUpdateArgs {
    #[argh(positional)]
    pub(crate) update_index: u64,

    /// output format: "json" or "porcelain"
    #[argh(option, short = 'o', default = "OutputFormat::Porcelain")]
    pub(crate) output_format: OutputFormat,
}

/// Get client state update at specified index.
pub(crate) fn get_client_state_update(
    _db: &impl DatabaseBackend,
    _args: GetClientStateUpdateArgs,
) -> Result<(), Box<dyn std::error::Error>> {
    Ok(())
}
