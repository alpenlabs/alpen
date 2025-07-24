use argh::FromArgs;
use strata_db::traits::DatabaseBackend;

use crate::cli::OutputFormat;

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "get-checkpoint")]
/// Get checkpoint
pub(crate) struct GetCheckpointArgs {
    #[argh(positional)]
    pub(crate) checkpoint_index: String,

    /// output format: "json" or "porcelain"
    #[argh(option, short = 'o', default = "OutputFormat::Porcelain")]
    pub(crate) output_format: OutputFormat,
}

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "get-checkpoints-summary")]
/// Get checkpoints summary
pub(crate) struct GetCheckpointsSummaryArgs {
    /// output format: "json" or "porcelain"
    #[argh(option, short = 'o', default = "OutputFormat::Porcelain")]
    pub(crate) output_format: OutputFormat,
}

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "get-epoch-summary")]
/// Get epoch summary
pub(crate) struct GetEpochSummaryArgs {
    #[argh(positional)]
    pub epoch_index: u64,

    /// output format: "json" or "porcelain"
    #[argh(option, short = 'o', default = "OutputFormat::Porcelain")]
    pub(crate) output_format: OutputFormat,
}

/// Get checkpoint details by index.
pub(crate) fn get_checkpoint(
    _db: &impl DatabaseBackend,
    _args: GetCheckpointArgs,
) -> Result<(), Box<dyn std::error::Error>> {
    Ok(())
}

/// Get summary of all checkpoints.
pub(crate) fn get_checkpoints_summary(
    _db: &impl DatabaseBackend,
    _args: GetCheckpointsSummaryArgs,
) -> Result<(), Box<dyn std::error::Error>> {
    Ok(())
}

/// Get epoch summary at specified index.
pub(crate) fn get_epoch_summary(
    _db: &impl DatabaseBackend,
    _args: GetEpochSummaryArgs,
) -> Result<(), Box<dyn std::error::Error>> {
    Ok(())
}
