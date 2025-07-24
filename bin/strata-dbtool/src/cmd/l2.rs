use argh::FromArgs;
use strata_cli_common::errors::DisplayedError;
use strata_db::traits::DatabaseBackend;

use crate::cli::OutputFormat;

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "get-l2-block")]
/// Get L2 block
pub(crate) struct GetL2BlockArgs {
    #[argh(positional)]
    pub(crate) block_id: String,

    /// output format: "json" or "porcelain"
    #[argh(option, short = 'o', default = "OutputFormat::Porcelain")]
    pub(crate) output_format: OutputFormat,
}

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "get-l2-summary")]
/// Get L2 summary
pub(crate) struct GetL2SummaryArgs {
    /// output format: "json" or "porcelain"
    #[argh(option, short = 'o', default = "OutputFormat::Porcelain")]
    pub(crate) output_format: OutputFormat,
}

/// Get L2 block by block ID.
pub(crate) fn get_l2_block(
    _db: &impl DatabaseBackend,
    _args: GetL2BlockArgs,
) -> Result<(), DisplayedError> {
    Ok(())
}

/// Get L2 summary - check all L2 blocks exist in database.
pub(crate) fn get_l2_summary(
    _db: &impl DatabaseBackend,
    _args: GetL2SummaryArgs,
) -> Result<(), DisplayedError> {
    Ok(())
}
