use argh::FromArgs;
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_db::traits::{BlockStatus, Database, L2BlockDatabase};

use super::{chainstate::get_latest_l2_write_batch, l1::get_l1_chain_tip};
use crate::cli::OutputFormat;

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
    db: &impl Database,
    _args: GetSyncinfoArgs,
) -> Result<(), DisplayedError> {
    let l2_db = db.l2_db();

    // Get L1 tip using helper function
    let (l1_tip_height, l1_tip_block_id) = get_l1_chain_tip(db)?;

    // Get L2 tip information
    let latest_write_batch = get_latest_l2_write_batch(db)
        .internal_error("Failed to get latest write batch")?
        .ok_or_else(|| {
            DisplayedError::InternalError(
                "Failed to get latest write batch".to_string(),
                Box::new(()),
            )
        })?;
    let top_level_state = latest_write_batch.toplevel_chainstate();
    let l2_tip_height = top_level_state.chain_tip_slot();
    let l2_tip_block_id = latest_write_batch.blockid();

    // Get L2 block status
    let l2_tip_block_status = l2_db
        .get_block_status(*l2_tip_block_id)
        .internal_error("Failed to get L2 tip block status")?
        .unwrap_or(BlockStatus::Unchecked);

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

    // Print in porcelain format
    println!("syncinfo.l1_tip_height {l1_tip_height}");
    println!("syncinfo.l1_tip_block_id {l1_tip_block_id:?}");
    println!("syncinfo.l2_tip_height {l2_tip_height}");
    println!("syncinfo.l2_tip_block_id {l2_tip_block_id:?}");
    println!("syncinfo.l2_tip_block_status {l2_tip_block_status:?}");
    println!("syncinfo.l2_finalized_block_id {l2_finalized_block_id:?}");
    println!("syncinfo.current_epoch {current_epoch}");
    println!("syncinfo.current_slot {current_slot}");
    println!("syncinfo.previous_block.slot {}", prev_block.slot());
    println!("syncinfo.previous_block.blkid {:?}", prev_block.blkid());
    println!("syncinfo.previous_epoch.epoch {}", prev_epoch.epoch());
    println!(
        "syncinfo.previous_epoch.last_slot {}",
        prev_epoch.last_slot()
    );
    println!(
        "syncinfo.previous_epoch.last_blkid {:?}",
        prev_epoch.last_blkid()
    );
    println!("syncinfo.finalized_epoch.epoch {}", finalized_epoch.epoch());
    println!(
        "syncinfo.finalized_epoch.last_slot {}",
        finalized_epoch.last_slot()
    );
    println!(
        "syncinfo.finalized_epoch.last_blkid {:?}",
        finalized_epoch.last_blkid()
    );
    println!("syncinfo.safe_block.height {}", safe_block.height());
    println!("syncinfo.safe_block.blkid {:?}", safe_block.blkid());
    Ok(())
}
