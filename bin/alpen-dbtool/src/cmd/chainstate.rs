use std::sync::Arc;

use clap::Args;
use hex::FromHex;
use strata_db::traits::{ChainstateDatabase, Database, L2BlockDatabase};
use strata_primitives::{buf::Buf32, l2::L2BlockId};
use strata_rocksdb::CommonDb;
use strata_state::{header::L2Header, state_op::WriteBatchEntry};

// use strata_state::header::L2Header;
use crate::errors::{DisplayableError, DisplayedError};

/// Arguments to get the chainstate with a specific write index.
#[derive(Args, Debug)]
pub(crate) struct GetChainstateArgs {
    /// Chainstate write index; defaults to the latest
    #[arg(value_name = "CHAINSTATE_WRITE_INDEX")]
    pub(crate) write_idx: Option<u64>,
}

/// Arguments to reset the chainstate to a specific L2 block.
#[derive(Args, Debug)]
pub(crate) struct ResetChainstateArgs {
    /// Target L2 block hash or number to roll back to.
    #[arg(value_name = "L2_BLOCK_ID")]
    pub(crate) block_id: String,
}

pub(crate) fn get_chainstate(
    db: Arc<CommonDb>,
    args: GetChainstateArgs,
) -> Result<(), DisplayedError> {
    let (chainstate_entry, write_idx) = get_chainstate_entry(db.clone(), args.write_idx)?;
    let (batch_info, _) = chainstate_entry.to_parts();
    let top_level_state = batch_info.new_toplevel_state();
    println!("Chainstate write index: {write_idx}");
    println!(
        "Chain tip: {}, current epoch: {}, epoch finishing: {}",
        top_level_state.chain_tip_slot(),
        top_level_state.cur_epoch(),
        top_level_state.is_epoch_finishing()
    );
    println!("Previous epoch: {:?}", top_level_state.prev_epoch());
    println!("Finalized epoch: {:?}", top_level_state.finalized_epoch());
    println!("L1 view: {:?}", top_level_state.l1_view());
    println!("Deposits table: {:?}", top_level_state.deposits_table());

    Ok(())
}

/// Reset the chainstate to a specific L2 block.
pub(crate) fn reset_chainstate(
    db: Arc<CommonDb>,
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
    let (chainstate_entry, latest_slot) = get_chainstate_entry(db.clone(), None)?;
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
    db: Arc<CommonDb>,
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
