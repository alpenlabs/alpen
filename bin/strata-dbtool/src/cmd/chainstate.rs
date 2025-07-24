use argh::FromArgs;
use strata_db::traits::DatabaseBackend;

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

    /// allow reverting blocks inside checkpointed epoch
    #[argh(switch, short = 'c')]
    pub(crate) revert_checkpointed_blocks: bool,
}

/// Get chainstate at specified block.
pub(crate) fn get_chainstate(
    _db: &impl DatabaseBackend,
    _args: GetChainstateArgs,
) -> Result<(), Box<dyn std::error::Error>> {
    Ok(())
}

/// Revert chainstate to specified block.
pub(crate) fn revert_chainstate(
    _db: &impl DatabaseBackend,
    _args: RevertChainstateArgs,
) -> Result<(), Box<dyn std::error::Error>> {
    Ok(())
}
