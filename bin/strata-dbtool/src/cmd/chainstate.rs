use argh::FromArgs;
use hex::FromHex;
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_db::traits::{ChainstateDatabase, Database, L2BlockDatabase};
use strata_primitives::{buf::Buf32, l2::L2BlockId, prelude::EpochCommitment};
use strata_state::{header::L2Header, state_op::WriteBatchEntry};

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
    previous_epoch: u64,
    previous_epoch_last_slot: u64,
    previous_epoch_last_block_id: &'a L2BlockId,
    finalized_epoch: u64,
    finalized_epoch_last_slot: u64,
    finalized_epoch_last_block_id: &'a L2BlockId,
}

pub(crate) fn get_chainstate(
    db: &impl Database,
    args: GetChainstateArgs,
) -> Result<(), DisplayedError> {
    let (chainstate_entry, write_idx) = get_chainstate_entry(db, args.write_idx)?;
    let (batch_info, _) = chainstate_entry.to_parts();
    let top_level_state = batch_info.new_toplevel_state();
    if args.output_format == OutputFormat::Json {
        let chainstate_info = ChainstateInfo {
            write_index: write_idx,
            chain_tip: top_level_state.chain_tip_slot(),
            current_epoch: top_level_state.cur_epoch(),
            is_epoch_finishing: top_level_state.is_epoch_finishing(),
            previous_epoch: top_level_state.prev_epoch().epoch(),
            previous_epoch_last_slot: top_level_state.prev_epoch().last_slot(),
            previous_epoch_last_block_id: top_level_state.prev_epoch().last_blkid(),
            finalized_epoch: top_level_state.finalized_epoch().epoch(),
            finalized_epoch_last_slot: top_level_state.finalized_epoch().last_slot(),
            finalized_epoch_last_block_id: top_level_state.finalized_epoch().last_blkid(),
        };
        println!(
            "{}",
            serde_json::to_string_pretty(&chainstate_info).unwrap()
        );
    } else {
        println!("Chainstate write index {write_idx}");
        println!("Chainstate chain tip {}", top_level_state.chain_tip_slot());
        println!("chainstate.current_epoch {}", top_level_state.cur_epoch());
        println!(
            "Chainstate epoch finishing {}",
            top_level_state.is_epoch_finishing()
        );
        println!(
            "chainstate previous epoch {:?}",
            top_level_state.prev_epoch()
        );
        println!(
            "Chainstate finalized epoch {:?}",
            top_level_state.finalized_epoch()
        );
        println!("Chainstate L1 view {:?}", top_level_state.l1_view());
        println!(
            "chainstate deposits table {:?}",
            top_level_state.deposits_table()
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
    // db.chain_state_db()
    //     .rollback_writes_to(target_slot)
    //     .internal_error("failed to reset chainstate")?;

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
