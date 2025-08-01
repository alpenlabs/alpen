use argh::FromArgs;
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_db::traits::{BlockStatus, ChainstateDatabase, Database, L2BlockDatabase};
use strata_primitives::l2::L2BlockId;
use strata_state::state_op::WriteBatchEntry;

use super::{checkpoint::get_latest_checkpoint_entry, l2::get_l2_block_slot};
use crate::{
    cli::OutputFormat,
    output::{chainstate::ChainstateInfo, output},
    utils::block_id::parse_l2_block_id,
};

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

/// Get the write batch for a specific L2 block.
pub(crate) fn get_l2_write_batch(
    db: &impl Database,
    block_id: L2BlockId,
) -> Result<Option<WriteBatchEntry>, DisplayedError> {
    let l2_block_slot = get_l2_block_slot(db, block_id)?;
    if let Some(l2_block_slot) = l2_block_slot {
        db.chain_state_db()
            .get_write_batch(l2_block_slot)
            .internal_error("Failed to get L2 write batch")
    } else {
        Ok(None)
    }
}

/// Get chainstate at specified block.
pub(crate) fn get_chainstate(
    db: &impl Database,
    args: GetChainstateArgs,
) -> Result<(), DisplayedError> {
    // Get block ID
    let block_id = parse_l2_block_id(&args.block_id)?;

    // Get the write batch for the specified block
    let Some(write_batch) = get_l2_write_batch(db, block_id)? else {
        return Err(DisplayedError::UserError(
            "L2 write batch not found".to_string(),
            Box::new(block_id),
        ));
    };

    let top_level_state = write_batch.toplevel_chainstate();

    // Get the block slot
    let block_slot = get_l2_block_slot(db, block_id)?.ok_or_else(|| {
        DisplayedError::UserError("L2 block with id not found".to_string(), Box::new(block_id))
    })?;

    let prev_epoch = top_level_state.prev_epoch();
    let finalized_epoch = top_level_state.finalized_epoch();
    let l1_view_state = top_level_state.l1_view();

    // Create the output data structure
    let chainstate_info = ChainstateInfo {
        block_id: &block_id,
        current_slot: block_slot,
        current_epoch: top_level_state.cur_epoch(),
        is_epoch_finishing: top_level_state.is_epoch_finishing(),
        previous_epoch: prev_epoch,
        finalized_epoch,
        l1_next_expected_height: l1_view_state.next_expected_height(),
        l1_safe_block_height: l1_view_state.safe_height(),
        l1_safe_block_blkid: l1_view_state.safe_blkid(),
    };

    // Use the output utility
    output(&chainstate_info, args.output_format)
}

/// Revert chainstate to specified block.
pub(crate) fn revert_chainstate(
    db: &impl Database,
    args: RevertChainstateArgs,
) -> Result<(), DisplayedError> {
    let target_block_id = parse_l2_block_id(&args.block_id)?;
    let target_slot = get_l2_block_slot(db, target_block_id)?.ok_or_else(|| {
        DisplayedError::UserError(
            "L2 block with id not found".to_string(),
            Box::new(target_block_id),
        )
    })?;

    // Get latest write batch to check finalized epoch constraints
    let latest_write_batch = get_latest_l2_write_batch(db)
        .internal_error("Failed to get latest write batch")?
        .ok_or_else(|| {
            DisplayedError::InternalError(
                "Failed to get latest write batch".to_string(),
                Box::new(()),
            )
        })?;

    let top_level_state = latest_write_batch.toplevel_chainstate();
    let finalized_slot = top_level_state.finalized_epoch().last_slot();
    let latest_slot = top_level_state.chain_tip_slot();

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
    println!("Revert chainstate checkpoint last slot {checkpoint_last_slot}");
    println!("Revert chainstate target slot {target_slot}");

    // Revert chainstate to target slot
    db.chain_state_db()
        .rollback_writes_to(target_slot)
        .internal_error(format!("Failed to rollback writes to {target_slot}"))?;

    for slot in target_slot + 1..=latest_slot {
        let l2_block_ids = db.l2_db().get_blocks_at_height(slot).unwrap_or_default();
        for block_id in l2_block_ids.iter() {
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
