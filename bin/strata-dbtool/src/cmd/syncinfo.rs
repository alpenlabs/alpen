use argh::FromArgs;
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_db::traits::{BlockStatus, ChainstateDatabase, Database, L1Database, L2BlockDatabase};
use strata_primitives::{
    l1::L1BlockId,
    l2::L2BlockId,
    prelude::{EpochCommitment, L1BlockCommitment},
};

use crate::cli::OutputFormat;

/// Show latest sync information
#[derive(FromArgs, Debug)]
#[argh(subcommand, name = "get-syncinfo")]
pub(crate) struct GetSyncinfoArgs {
    /// output format: "json" or "porcelain"
    #[argh(option, short = 'f', default = "OutputFormat::Porcelain")]
    pub(crate) output_format: OutputFormat,
}

/// Sync information displayed to the user
#[derive(serde::Serialize)]
struct SyncInfo<'a> {
    l1_tip_height: u64,
    l1_tip_block_id: &'a L1BlockId,
    l2_tip_height: u64,
    l2_tip_block_id: &'a L2BlockId,
    l2_tip_block_status: &'a BlockStatus,
    l2_finalized_block_id: &'a L2BlockId,
    current_epoch: u64,
    previous_epoch: &'a EpochCommitment,
    finalized_epoch: &'a EpochCommitment,
    safe_block_id: &'a L1BlockCommitment,
}

/// Show the latest sync information.
pub(crate) fn get_syncinfo(
    db: &impl Database,
    args: GetSyncinfoArgs,
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

    let l2_block_status = db
        .l2_db()
        .get_block_status(l2_block_id)
        .ok()
        .flatten()
        .unwrap_or(BlockStatus::Unchecked);
    let l2_block_height = batch_info.new_toplevel_state().chain_tip_slot();
    let top_level_state = batch_info.new_toplevel_state();

    // Print sync information
    if args.output_format == OutputFormat::Json {
        let syncinfo = SyncInfo {
            l1_tip_height,
            l1_tip_block_id: &l1_tip_block_id,
            l2_tip_height: l2_block_height,
            l2_tip_block_id: &l2_block_id,
            l2_tip_block_status: &l2_block_status,
            l2_finalized_block_id: batch_info
                .new_toplevel_state()
                .finalized_epoch()
                .last_blkid(),
            current_epoch: top_level_state.cur_epoch(),
            previous_epoch: top_level_state.prev_epoch(),
            finalized_epoch: top_level_state.finalized_epoch(),
            safe_block_id: &top_level_state.l1_view().get_safe_block(),
        };
        println!("{}", serde_json::to_string_pretty(&syncinfo).unwrap());
    } else {
        println!("syncinfo.l1_tip.height: {l1_tip_height}");
        println!("syncinfo.l1_tip.blkid {l1_tip_block_id:?}");
        println!("syncinfo.l2_tip.height: {l2_block_height}");
        println!("syncinfo.l2_tip.blkid {l2_block_id:?}");
        println!("syncinfo.l2_tip.block_status {l2_block_status:?}");

        let top_level_state = batch_info.new_toplevel_state();
        println!(
            "syncinfo.top_level_state.current_epoch: {}",
            top_level_state.cur_epoch()
        );
        println!(
            "syncinfo.top_level_state.current_slot: {}",
            top_level_state.chain_tip_slot()
        );

        let prev_block = top_level_state.prev_block();
        println!(
            "syncinfo.top_level_state.prev_block.height: {}",
            prev_block.slot()
        );
        println!(
            "syncinfo.top_level_state.prev_block.blkid: {:?}",
            prev_block.blkid()
        );

        let prev_epoch = top_level_state.prev_epoch();
        println!(
            "syncinfo.top_level_state.prev_epoch.epoch: {:?}",
            prev_epoch.epoch()
        );
        println!(
            "syncinfo.top_level_state.prev_epoch.last_slot: {:?}",
            prev_epoch.last_slot()
        );
        println!(
            "syncinfo.top_level_state.prev_epoch.last_blkid: {:?}",
            prev_epoch.last_blkid()
        );

        let finalized_epoch = top_level_state.finalized_epoch();
        println!(
            "syncinfo.top_level_state.finalized_epoch.epoch: {:?}",
            finalized_epoch.epoch()
        );
        println!(
            "syncinfo.top_level_state.finalized_epoch.last_slot: {:?}",
            finalized_epoch.last_slot()
        );
        println!(
            "syncinfo.top_level_state.finalized_epoch.last_blkid: {:?}",
            finalized_epoch.last_blkid()
        );

        let l1_safe_block = top_level_state.l1_view().get_safe_block();
        println!(
            "syncinfo.top_level_state.l1_view.safe_block.height: {}",
            l1_safe_block.height()
        );
        println!(
            "syncinfo.top_level_state.l1_view.safe_block.blkid: {:?}",
            l1_safe_block.blkid()
        );
    }

    Ok(())
}
