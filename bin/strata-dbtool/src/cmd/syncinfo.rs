use argh::FromArgs;
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_db_types::traits::{BlockStatus, DatabaseBackend, L2BlockDatabase};

use super::{
    chainstate::get_latest_l2_write_batch,
    l1::get_l1_chain_tip,
    l2::{get_chain_tip_block_id, get_chain_tip_slot},
};
use crate::{
    cli::OutputFormat,
    output::{output, syncinfo::SyncInfo},
};

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "get-syncinfo")]
/// Get sync info
pub(crate) struct GetSyncinfoArgs {
    /// output format: "porcelain" (default) or "json"
    #[argh(option, short = 'o', default = "OutputFormat::Porcelain")]
    pub(crate) output_format: OutputFormat,
}

/// Show the latest sync information.
pub(crate) fn get_syncinfo(
    db: &impl DatabaseBackend,
    args: GetSyncinfoArgs,
) -> Result<(), DisplayedError> {
    let l2_db = db.l2_db();

    // Get L1 tip
    let (l1_tip_height, l1_tip_block_id) = get_l1_chain_tip(db)?;

    // Get L2 tip
    let l2_tip_block_id = get_chain_tip_block_id(db)?;

    // Get L2 tip slot
    let l2_tip_height = get_chain_tip_slot(db)?;

    // Get L2 block status
    let l2_tip_block_status = l2_db
        .get_block_status(l2_tip_block_id)
        .internal_error("Failed to get L2 tip block status")?
        .unwrap_or(BlockStatus::Unchecked);

    // Get latest write batch to understand current state
    let write_batch = get_latest_l2_write_batch(db)?;
    let top_level_state = write_batch.new_toplevel_state();

    // Get previous block info
    let prev_block = top_level_state.prev_block();

    // Get epoch info
    let prev_epoch = top_level_state.prev_epoch();
    let finalized_epoch = top_level_state.finalized_epoch();

    // Get current epoch and slot
    let current_epoch = top_level_state.cur_epoch();
    let current_slot = top_level_state.chain_tip_slot();

    // Get L2 finalized block ID
    let l2_finalized_block_id = finalized_epoch.last_blkid();

    // Get L1 safe block
    let safe_block = top_level_state.l1_view().get_safe_block();

    // Create the output data structure
    let sync_info = SyncInfo {
        l1_tip_height,
        l1_tip_block_id: &l1_tip_block_id,
        l2_tip_height,
        l2_tip_block_id: &l2_tip_block_id,
        l2_tip_block_status: &l2_tip_block_status,
        l2_finalized_block_id,
        current_epoch: current_epoch as u64,
        current_slot,
        previous_block: prev_block,
        previous_epoch: prev_epoch,
        finalized_epoch,
        safe_block: &safe_block,
    };

    // Use the output utility
    output(&sync_info, args.output_format)
}
