use argh::FromArgs;
use strata_db::traits::DatabaseBackend;

use crate::cli::OutputFormat;

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "get-l1-manifest")]
/// Get L1 manifest
pub(crate) struct GetL1ManifestArgs {
    #[argh(positional)]
    pub(crate) block_id: String,

    /// output format: "json" or "porcelain"
    #[argh(option, short = 'o', default = "OutputFormat::Porcelain")]
    pub(crate) output_format: OutputFormat,
}

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "get-l1-summary")]
/// Get L1 summary
pub(crate) struct GetL1SummaryArgs {
    /// output format: "json" or "porcelain"
    #[argh(option, short = 'o', default = "OutputFormat::Porcelain")]
    pub(crate) output_format: OutputFormat,
}

/// Get L1 manifest by block ID.
pub(crate) fn get_l1_manifest(
    _db: &impl DatabaseBackend,
    _args: GetL1ManifestArgs,
) -> Result<(), Box<dyn std::error::Error>> {
    Ok(())
}

/// Get L1 summary - check all L1 block manifests exist in database.
pub(crate) fn get_l1_summary(
    _db: &impl DatabaseBackend,
    _args: GetL1SummaryArgs,
) -> Result<(), Box<dyn std::error::Error>> {
    Ok(())
}
