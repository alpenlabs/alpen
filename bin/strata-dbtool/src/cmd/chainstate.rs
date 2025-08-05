use argh::FromArgs;
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_consensus_logic::chain_worker_context::conv_blkid_to_slot_wb_id;
use strata_db::{
    chainstate::ChainstateDatabase,
    traits::{BlockStatus, DatabaseBackend, L2BlockDatabase},
};
use strata_primitives::l2::L2BlockId;
use strata_state::state_op::WriteBatch;

use super::{
    checkpoint::get_latest_checkpoint_entry,
    l2::{get_highest_l2_slot, get_l2_block_slot, get_latest_l2_block_id},
};
use crate::{cli::OutputFormat, utils::block_id::parse_l2_block_id};

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "get-chainstate")]
/// Get chainstate at specified block
pub(crate) struct GetChainstateArgs {
    /// L2 block id
    #[argh(positional)]
    pub(crate) block_id: String,

    /// output format: "porcelain" (default) or "json"
    #[argh(option, short = 'o', default = "OutputFormat::Porcelain")]
    pub(crate) output_format: OutputFormat,
}

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "revert-chainstate")]
/// Revert chainstate to specified block
pub(crate) struct RevertChainstateArgs {
    /// target L2 block id
    #[argh(positional)]
    pub(crate) block_id: String,

    /// delete blocks after target block
    #[argh(switch, short = 'd')]
    pub(crate) delete_blocks: bool,

    /// allow reverting blocks inside checkpointed epoch
    #[argh(switch, short = 'c')]
    pub(crate) revert_checkpointed_blocks: bool,
}

/// Get the write batch for a specific L2 block.
///
/// This gets the write batch associated with the specified block ID.
pub(crate) fn get_l2_write_batch(
    db: &impl DatabaseBackend,
    block_id: L2BlockId,
) -> Result<Option<WriteBatch>, DisplayedError> {
    // Convert block ID to write batch ID
    let write_batch_id = conv_blkid_to_slot_wb_id(block_id);

    db.chain_state_db()
        .get_write_batch(write_batch_id)
        .internal_error("Failed to get L2 write batch")
}

/// Get the write batch for the latest L2 block.
///
/// This gets the write batch associated with the highest slot block in the database.
pub(crate) fn get_latest_l2_write_batch(
    db: &impl DatabaseBackend,
) -> Result<strata_state::state_op::WriteBatch, DisplayedError> {
    let block_id = get_latest_l2_block_id(db)?;
    get_l2_write_batch(db, block_id)?.ok_or_else(|| {
        DisplayedError::InternalError("L2 write batch not found".to_string(), Box::new(block_id))
    })
}

/// Get chainstate at specified block.
pub(crate) fn get_chainstate(
    db: &impl DatabaseBackend,
    args: GetChainstateArgs,
) -> Result<(), DisplayedError> {
    // Get block ID
    let block_id = parse_l2_block_id(&args.block_id)?;

    // Get the write batch for the specified block
    let write_batch = get_l2_write_batch(db, block_id)?.ok_or_else(|| {
        DisplayedError::UserError("L2 write batch not found".to_string(), Box::new(block_id))
    })?;
    let top_level_state = write_batch.new_toplevel_state();

    // Get the block slot
    let block_slot = get_l2_block_slot(db, block_id)?.ok_or_else(|| {
        DisplayedError::UserError("L2 block with id not found".to_string(), Box::new(block_id))
    })?;

    let prev_epoch = top_level_state.prev_epoch();
    let finalized_epoch = top_level_state.finalized_epoch();
    let l1_view_state = top_level_state.l1_view();

    // Print in porcelain format
    println!("chainstate.block_id {block_id:?}");
    println!("chainstate.current_slot {block_slot}");
    println!("chainstate.current_epoch {}", top_level_state.cur_epoch());
    println!(
        "chainstate.is_epoch_finishing {}",
        top_level_state.is_epoch_finishing()
    );
    println!("chainstate.previous_epoch.epoch {}", prev_epoch.epoch());
    println!(
        "chainstate.previous_epoch.last_slot {}",
        prev_epoch.last_slot()
    );
    println!(
        "chainstate.previous_epoch.last_blkid {:?}",
        prev_epoch.last_blkid()
    );
    println!(
        "chainstate.finalized_epoch.epoch {}",
        finalized_epoch.epoch()
    );
    println!(
        "chainstate.finalized_epoch.last_slot {}",
        finalized_epoch.last_slot()
    );
    println!(
        "chainstate.finalized_epoch.last_blkid {:?}",
        finalized_epoch.last_blkid()
    );
    println!(
        "chainstate.l1_next_expected_height {}",
        l1_view_state.next_expected_height()
    );
    println!(
        "chainstate.l1_safe_block_height {}",
        l1_view_state.safe_height()
    );
    println!(
        "chainstate.l1_safe_block_blkid {:?}",
        l1_view_state.safe_blkid()
    );

    Ok(())
}

/// Revert chainstate to specified block.
pub(crate) fn revert_chainstate(
    db: &impl DatabaseBackend,
    args: RevertChainstateArgs,
) -> Result<(), DisplayedError> {
    let target_block_id = parse_l2_block_id(&args.block_id)?;
    let target_slot = get_l2_block_slot(db, target_block_id)?.ok_or_else(|| {
        DisplayedError::UserError(
            "L2 block with id not found".to_string(),
            Box::new(target_block_id),
        )
    })?;

    // Get the latest slot
    let latest_slot = get_highest_l2_slot(db)?;

    // Get latest write batch to check finalized epoch constraints
    let write_batch = get_latest_l2_write_batch(db)?;
    let top_level_state = write_batch.new_toplevel_state();
    let finalized_slot = top_level_state.finalized_epoch().last_slot();

    if target_slot < finalized_slot {
        return Err(DisplayedError::UserError(
            "Target block is inside finalized epoch".to_string(),
            Box::new(target_block_id),
        ));
    }

    // Check if target block is inside checkpointed epoch
    let latest_checkpoint_entry = get_latest_checkpoint_entry(db)?;
    let checkpoint_last_slot = latest_checkpoint_entry
        .checkpoint
        .batch_info()
        .l2_range
        .1
        .slot();

    if !args.revert_checkpointed_blocks && target_slot < checkpoint_last_slot {
        return Err(DisplayedError::UserError(
            "Target block is inside checkpointed epoch".to_string(),
            Box::new(target_block_id),
        ));
    }

    println!("Chainstate latest slot {latest_slot}");
    println!("Chainstate finalized slot {finalized_slot}");
    println!("Latest checkpointed slot {checkpoint_last_slot}");
    println!("Revert chainstate target slot {target_slot}");

    // Now delete write batches and optionally blocks
    for slot in target_slot + 1..=latest_slot {
        let l2_block_ids = db.l2_db().get_blocks_at_height(slot).unwrap_or_default();
        for block_id in l2_block_ids.iter() {
            // Convert block ID to write batch ID
            let write_batch_id = conv_blkid_to_slot_wb_id(*block_id);

            // Check if write batch exists before deleting
            let write_batch_exists = db
                .chain_state_db()
                .get_write_batch(write_batch_id)
                .internal_error("Failed to check write batch existence")?
                .is_some();

            if write_batch_exists {
                println!("Revert chainstate deleting write batch {block_id:?} {slot}");
                db.chain_state_db()
                    .del_write_batch(write_batch_id)
                    .internal_error(format!(
                        "Failed to delete write batch for block {}",
                        *block_id
                    ))?;
            } else {
                println!("Revert chainstate no write batch found {block_id:?} {slot}");
                continue;
            }

            // Mark the status to unchecked
            println!("Revert chainstate marking block unchecked {block_id:?}");
            db.l2_db()
                .set_block_status(*block_id, BlockStatus::Unchecked)
                .internal_error(format!(
                    "Failed to update status for block with id {}",
                    *block_id
                ))?;

            // Delete blocks if requested
            if args.delete_blocks {
                println!("Revert chainstate deleting block {block_id:?}");
                db.l2_db()
                    .del_block_data(*block_id)
                    .internal_error(format!("Failed to delete block with id {}", *block_id))?;
            }
        }
    }

    println!("Revert chainstate completed");
    Ok(())
}
