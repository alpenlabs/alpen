use argh::FromArgs;
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_db::traits::{ChainstateDatabase, Database};
use strata_state::state_op::WriteBatchEntry;

use crate::cli::OutputFormat;

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "get-chainstate")]
/// Get chainstate at specified block
pub(crate) struct GetChainstateArgs {
    #[argh(positional)]
    pub block_id: String,

    /// output format: "json" or "porcelain"
    #[argh(option, short = 'o', default = "OutputFormat::Porcelain")]
    pub(crate) output_format: OutputFormat,
}

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "revert-chainstate")]
/// Revert chainstate to specified block
pub(crate) struct RevertChainstateArgs {
    #[argh(positional)]
    pub(crate) block_id: String,

    /// delete blocks after target block
    #[argh(switch, short = 'd')]
    pub(crate) delete_blocks: bool,
}

/// Get the write batch for the latest L2 block.
///
/// This gets the write batch associated with the highest slot block in the database.
pub(crate) fn get_latest_l2_write_batch(
    db: &impl Database,
) -> Result<Option<WriteBatchEntry>, DisplayedError> {
    let latest_write_batch_idx = db
        .chain_state_db()
        .get_last_write_idx()
        .internal_error("Failed to get last write batch index")?;

    db.chain_state_db()
        .get_write_batch(latest_write_batch_idx)
        .internal_error("Failed to get last write batch")
}

/// Get chainstate at specified block.
pub(crate) fn get_chainstate(
    _db: &impl Database,
    _args: GetChainstateArgs,
) -> Result<(), DisplayedError> {
    Ok(())
}

/// Revert chainstate to specified block.
pub(crate) fn revert_chainstate(
    _db: &impl Database,
    _args: RevertChainstateArgs,
) -> Result<(), DisplayedError> {
    Ok(())
}
