use argh::FromArgs;
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_db::traits::{BlockStatus, ChainstateDatabase, Database, L1Database, L2BlockDatabase};

use crate::cli::OutputFormat;

/// Show latest sync information
#[derive(FromArgs, Debug)]
#[argh(subcommand, name = "get-syncinfo")]
pub(crate) struct GetSyncinfoArgs {
    /// output format: "json" or "porcelain"
    #[argh(option, short = 'f', default = "OutputFormat::Porcelain")]
    pub(crate) output_format: OutputFormat,
}

/// Show the latest sync information.
pub(crate) fn get_syncinfo(
    db: &impl Database,
    _args: GetSyncinfoArgs,
) -> Result<(), DisplayedError> {
    let l1_db = db.l1_db();
    let (l1_tip_height, l1_tip_block_id) = l1_db
        .get_canonical_chain_tip()
        .internal_error("Failed to read L1 tip")?
        .expect("valid L1 tip");

    let last_l2_write_idx = db
        .chain_state_db()
        .get_last_write_idx()
        .internal_error("Failed to fetch latest chainstate write index")?;

    let chainstate_entry = db
        .chain_state_db()
        .get_write_batch(last_l2_write_idx)
        .internal_error("Failed to fetch chainstate entry")?
        .expect("valid entry");
    let (batch_info, l2_block_id) = chainstate_entry.to_parts();

    let l2_block_status = db.l2_db().get_block_status(l2_block_id).ok().flatten();
    let l2_block_height = batch_info.new_toplevel_state().chain_tip_slot();

    // Show sync information
    println!("L1 tip: {l1_tip_height}, {l1_tip_block_id:?}");

    println!(
        "L2 height: {}, tip: {:?} ({:?})",
        l2_block_height,
        l2_block_id,
        l2_block_status.unwrap_or(BlockStatus::Unchecked)
    );
    println!(
        "Finalized block id: {:?}",
        batch_info
            .new_toplevel_state()
            .finalized_epoch()
            .last_blkid()
    );
    println!(
        "Current epoch: {:?}",
        batch_info.new_toplevel_state().cur_epoch()
    );
    println!(
        "Previous epoch: {:?}",
        batch_info.new_toplevel_state().prev_epoch()
    );
    println!(
        "Finalized epoch: {:?}",
        batch_info.new_toplevel_state().finalized_epoch()
    );
    println!(
        "L1 safe block: {:?}",
        batch_info.new_toplevel_state().l1_view().get_safe_block()
    );

    Ok(())
}
