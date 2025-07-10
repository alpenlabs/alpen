use argh::FromArgs;
use hex::FromHex;
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_db::traits::{BlockStatus, ChainstateDatabase, Database, L2BlockDatabase};
use strata_primitives::{buf::Buf32, l2::L2BlockId, prelude::EpochCommitment};
use strata_state::{header::L2Header, l1::L1BlockId, state_op::WriteBatchEntry};

use crate::cli::OutputFormat;

/// Shows the chainstate at the provided index
#[derive(FromArgs, Debug)]
#[argh(subcommand, name = "get-chainstate")]
pub(crate) struct GetChainstateArgs {
    /// chainstate write index; defaults to the latest
    #[argh(positional)]
    pub(crate) write_idx: Option<u64>,

    /// output format: "json" or "porcelain"
    #[argh(option, short = 'f', default = "OutputFormat::Porcelain")]
    pub(crate) output_format: OutputFormat,
}

/// Resets the chainstate to a specific L2 block
#[derive(FromArgs, Debug)]
#[argh(subcommand, name = "reset-chainstate")]
pub(crate) struct ResetChainstateArgs {
    /// target L2 block id
    #[argh(positional)]
    pub(crate) block_id: String,

    /// clear status of blocks after target block
    #[argh(switch, short = 'u')]
    pub(crate) update_block_status: bool,

    /// delete blocks after target block
    #[argh(switch, short = 'd')]
    pub(crate) delete_blocks: bool,
}

/// Chainstate information displayed to the user
#[derive(serde::Serialize)]
struct ChainstateInfo<'a> {
    write_index: u64,
    chain_tip: u64,
    current_epoch: u64,
    is_epoch_finishing: bool,
    previous_epoch: &'a EpochCommitment,
    finalized_epoch: &'a EpochCommitment,
    l1_next_expected_height: u64,
    l1_safe_block_height: u64,
    l1_safe_block_blkid: &'a L1BlockId,
}

pub(crate) fn get_chainstate(
    db: &impl Database,
    args: GetChainstateArgs,
) -> Result<(), DisplayedError> {
    let (chainstate_entry, write_idx) = get_chainstate_entry(db, args.write_idx)?;
    let (batch_info, _) = chainstate_entry.to_parts();
    let top_level_state = batch_info.new_toplevel_state();
    let prev_epoch = top_level_state.prev_epoch();
    let finalized_epoch = top_level_state.finalized_epoch();
    let l1_view_state = top_level_state.l1_view();

    if args.output_format == OutputFormat::Json {
        let chainstate_info = ChainstateInfo {
            write_index: write_idx,
            chain_tip: top_level_state.chain_tip_slot(),
            current_epoch: top_level_state.cur_epoch(),
            is_epoch_finishing: top_level_state.is_epoch_finishing(),
            previous_epoch: prev_epoch,
            finalized_epoch,
            l1_next_expected_height: l1_view_state.next_expected_height(),
            l1_safe_block_height: l1_view_state.safe_height(),
            l1_safe_block_blkid: l1_view_state.safe_blkid(),
        };
        println!(
            "{}",
            serde_json::to_string_pretty(&chainstate_info).unwrap()
        );
    } else {
        println!("chainstate.write_index: {write_idx}");
        println!("chainstate.chain_tip: {}", top_level_state.chain_tip_slot());
        println!("chainstate.current_epoch {}", top_level_state.cur_epoch());
        println!(
            "chainstate.is_epoch_finishing: {}",
            top_level_state.is_epoch_finishing()
        );

        let prev_epoch = top_level_state.prev_epoch();
        println!("chainstate.prev_epoch.epoch: {:?}", prev_epoch.epoch());
        println!(
            "chainstate.prev_epoch.last_slot: {:?}",
            prev_epoch.last_slot()
        );
        println!(
            "chainstate.prev_epoch.last_blkid: {:?}",
            prev_epoch.last_blkid()
        );

        let finalized_epoch = top_level_state.finalized_epoch();
        println!(
            "chainstate.finalized_epoch.epoch: {:?}",
            finalized_epoch.epoch()
        );
        println!(
            "chainstate.finalized_epoch.last_slot: {:?}",
            finalized_epoch.last_slot()
        );
        println!(
            "chainstate.finalized_epoch.last_blkid: {:?}",
            finalized_epoch.last_blkid()
        );

        let l1_view = top_level_state.l1_view();
        println!(
            "chainstate.l1_view.next_expected_height: {}",
            l1_view.next_expected_height()
        );
        println!(
            "chainstate.l1_view.safe_block.height: {}",
            l1_view.safe_height()
        );
        println!(
            "chainstate.l1_view.safe_block.blkid: {:?}",
            l1_view.safe_blkid()
        );
    }
    Ok(())
}

/// Reset the chainstate to a specific L2 block.
pub(crate) fn reset_chainstate(
    db: &impl Database,
    args: ResetChainstateArgs,
) -> Result<(), DisplayedError> {
    let hex_str = args.block_id.strip_prefix("0x").unwrap_or(&args.block_id);
    if hex_str.len() != 64 {
        return Err(DisplayedError::UserError(
            "Block-id must be 32-byte / 64-char hex".into(),
            Box::new(args.block_id.to_owned()),
        ));
    }

    let bytes: [u8; 32] =
        <[u8; 32]>::from_hex(hex_str).user_error(format!("Invalid 32-byte hex {hex_str}"))?;
    let target_block_id: L2BlockId = Buf32::from(bytes).into();
    let target_block_data = db
        .l2_db()
        .get_block_data(target_block_id)
        .internal_error("Failed to read block data")?
        .ok_or_else(|| {
            DisplayedError::UserError(
                "block with id not found".to_string(),
                Box::new(target_block_id),
            )
        })?;
    let target_block_height = target_block_data.header().slot();
    // It seems write index is the same as the slot number
    let (chainstate_entry, latest_slot) = get_chainstate_entry(db, None)?;
    let (batch_info, latest_block_id) = chainstate_entry.to_parts();

    let finalized_height = batch_info
        .new_toplevel_state()
        .finalized_epoch()
        .last_slot();

    if target_block_height < finalized_height {
        return Err(DisplayedError::UserError(
            "Target block is inside finalized epoch".to_string(),
            Box::new(target_block_id),
        ));
    }

    // Determine the slot to reset to
    let mut target_slot = latest_slot - 1;
    if target_block_id != latest_block_id {
        for entry_idx in (0..latest_slot).rev() {
            let entry = db
                .chain_state_db()
                .get_write_batch(entry_idx)
                .internal_error("Failed to fetch chainstate entry")?
                .expect("valid entry");

            let (_, block_id) = entry.to_parts();
            if block_id == target_block_id {
                target_slot = entry_idx;
                break;
            }
        }
    }

    println!("Resetting chainstate to slot {target_slot}");
    // Reset chainstate to the target slot
    db.chain_state_db()
        .rollback_writes_to(target_slot)
        .internal_error("failed to reset chainstate")?;

    // Additional actions
    if args.update_block_status || args.delete_blocks {
        for slot in target_slot + 1..latest_slot {
            let l2_block_ids = db.l2_db().get_blocks_at_height(slot).unwrap_or_default();
            for id in l2_block_ids.iter() {
                // Mark the status to unchecked
                if args.update_block_status {
                    println!("Marking block {id:?} as unchecked");
                    db.l2_db()
                        .set_block_status(*id, BlockStatus::Unchecked)
                        .internal_error(format!(
                            "Failed to update status for block with id {}",
                            *id
                        ))?;
                }
                // Delete blocks
                if args.delete_blocks {
                    println!("Deleting block {id:?}");
                    db.l2_db()
                        .del_block_data(*id)
                        .internal_error(format!("Failed to delete block with id {}", *id))?;
                }
            }
        }
    }

    Ok(())
}

/// Get the chainstate write batch entry from the database.
///
/// If `update_idx` is None, gets the latest chainstate write batch entry.
pub(super) fn get_chainstate_entry(
    db: &impl Database,
    update_idx: Option<u64>,
) -> Result<(WriteBatchEntry, u64), DisplayedError> {
    let chainstate_db = db.chain_state_db();
    let write_idx = update_idx.unwrap_or(
        chainstate_db
            .get_last_write_idx()
            .internal_error("Failed to fetch latest chainstate write index")?,
    );

    let chainstate_entry = db
        .chain_state_db()
        .get_write_batch(write_idx)
        .internal_error("Failed to fetch chainstate entry")?
        .expect("valid entry");

    Ok((chainstate_entry, write_idx))
}
