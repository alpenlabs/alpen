use argh::FromArgs;
use strata_db::traits::DatabaseBackend;

use crate::cli::OutputFormat;

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "get-sync-event")]
/// Get sync event
pub(crate) struct GetSyncEventArgs {
    #[argh(positional)]
    pub(crate) event_index: String,

    /// output format: "json" or "porcelain"
    #[argh(option, short = 'o', default = "OutputFormat::Porcelain")]
    pub(crate) output_format: OutputFormat,
}

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "get-sync-events-summary")]
/// Get sync events summary
pub(crate) struct GetSyncEventsSummaryArgs {
    /// output format: "json" or "porcelain"
    #[argh(option, short = 'o', default = "OutputFormat::Porcelain")]
    pub(crate) output_format: OutputFormat,
}

/// Get sync event details by ID.
pub(crate) fn get_sync_event(
    _db: &impl DatabaseBackend,
    _args: GetSyncEventArgs,
) -> Result<(), Box<dyn std::error::Error>> {
    Ok(())
}

/// Get summary of all sync events.
pub(crate) fn get_sync_events_summary(
    _db: &impl DatabaseBackend,
    _args: GetSyncEventsSummaryArgs,
) -> Result<(), Box<dyn std::error::Error>> {
    Ok(())
}
