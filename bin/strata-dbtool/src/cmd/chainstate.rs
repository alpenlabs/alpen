use argh::FromArgs;
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_db::traits::{
    BlockStatus, ChainstateDatabase, CheckpointDatabase, Database, L1BroadcastDatabase,
    L1WriterDatabase, L2BlockDatabase,
};
use strata_primitives::l2::L2BlockId;
use strata_state::state_op::WriteBatchEntry;

use super::{
    checkpoint::get_latest_checkpoint_entry,
    l2::{get_l2_block_slot, get_l2_block_slot_and_epoch},
};
use crate::{
    cli::OutputFormat,
    db::CommonDbBackend,
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

    /// force execution (without this flag, only a dry run is performed)
    #[argh(switch, short = 'f')]
    pub(crate) force: bool,
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
    db: &CommonDbBackend<impl Database, impl L1BroadcastDatabase, impl L1WriterDatabase>,
    args: RevertChainstateArgs,
) -> Result<(), DisplayedError> {
    let dry_run = !args.force;
    let core_db = &db.core;
    let target_block_id = parse_l2_block_id(&args.block_id)?;
    let (target_slot, target_epoch) = get_l2_block_slot_and_epoch(&db.core, target_block_id)?
        .ok_or_else(|| {
            DisplayedError::UserError(
                "L2 block with id not found".to_string(),
                Box::new(target_block_id),
            )
        })?;

    // Get latest write batch to check finalized epoch constraints
    let latest_write_batch = get_latest_l2_write_batch(core_db)
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
    let latest_checkpoint_entry = get_latest_checkpoint_entry(core_db)?;
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

    // Get the target block's write batch to check if target slot is epoch-finishing
    let target_write_batch = get_l2_write_batch(core_db, target_block_id)?.ok_or_else(|| {
        DisplayedError::UserError(
            "Target L2 write batch not found".to_string(),
            Box::new(target_block_id),
        )
    })?;
    let target_top_level_state = target_write_batch.toplevel_chainstate();
    let target_slot_is_terminal = target_top_level_state.is_epoch_finishing();

    println!("Chainstate latest slot {latest_slot}");
    println!("Chainstate finalized slot {finalized_slot}");
    println!("Latest checkpointed slot {checkpoint_last_slot}");
    println!("Revert chainstate target slot {target_slot}");
    println!("Target slot is epoch finishing: {target_slot_is_terminal}");
    println!();

    // Check if there are any blocks to revert
    if target_slot >= latest_slot {
        if dry_run {
            println!("========================================");
            println!("DRY RUN SUMMARY - No changes were made");
            println!("========================================");
            println!();
            println!("No changes would be made.");
            println!();
            println!("Target slot ({target_slot}) is at or after the latest slot ({latest_slot}).");
            println!("Nothing to revert.");
        } else {
            println!(
                "NOTE: Target slot ({target_slot}) is at or after the latest slot ({latest_slot})."
            );
            println!("Nothing to revert.");
        }
        return Ok(());
    }

    // Collect blocks and write batches to be modified/deleted
    let mut write_batches_to_delete = Vec::new();
    let mut blocks_to_mark_unchecked = Vec::new();
    let mut blocks_to_delete = Vec::new();

    for slot in target_slot + 1..=latest_slot {
        let l2_block_ids = core_db
            .l2_db()
            .get_blocks_at_height(slot)
            .unwrap_or_default();

        for block_id in l2_block_ids.iter() {
            // Write batches are indexed by slot in the release branch
            write_batches_to_delete.push((slot, *block_id));
            blocks_to_mark_unchecked.push(*block_id);

            if args.delete_blocks {
                blocks_to_delete.push(*block_id);
            }

            if !dry_run {
                // Mark the status to unchecked
                println!("Revert chainstate marking block unchecked {block_id:?}");
                core_db
                    .l2_db()
                    .set_block_status(*block_id, BlockStatus::Unchecked)
                    .internal_error(format!(
                        "Failed to update status for block with id {}",
                        *block_id
                    ))?;

                // Delete blocks if requested
                if args.delete_blocks {
                    println!("Revert chainstate deleting block {block_id:?}");
                    core_db
                        .l2_db()
                        .del_block_data(*block_id)
                        .internal_error(format!("Failed to delete block with id {}", *block_id))?;
                }
            }
        }
    }

    // Rollback chainstate writes (this deletes write batches from target_slot + 1 onwards)
    if !dry_run {
        core_db
            .chain_state_db()
            .rollback_writes_to(target_slot)
            .internal_error(format!("Failed to rollback writes to {target_slot}"))?;
    }

    // Determine first epoch to clean up
    // - If target_slot is terminal: target_epoch is complete, start cleaning from next epoch
    // - If target_slot is not terminal: target_epoch is incomplete, include it in cleanup
    let first_epoch_to_clean = if target_slot_is_terminal {
        target_epoch + 1
    } else {
        target_epoch
    };

    let latest_checkpoint_epoch = latest_checkpoint_entry.checkpoint.batch_info().epoch;

    // Check if there are checkpoints to clean
    let needs_checkpoint_cleanup = first_epoch_to_clean <= latest_checkpoint_epoch;

    // Check if there are epoch summaries to clean (may exist beyond latest checkpoint)
    let last_summarized_epoch = core_db
        .checkpoint_db()
        .get_last_summarized_epoch()
        .internal_error("Failed to get last summarized epoch")?;

    let needs_epoch_summary_cleanup = last_summarized_epoch
        .map(|last_epoch| first_epoch_to_clean <= last_epoch)
        .unwrap_or(false);

    let mut checkpoints_to_delete = Vec::new();
    let mut epoch_summaries_to_delete = Vec::new();

    // Note: We intentionally do NOT delete L1 related stuff (writer entries such as
    // intents/payloads or broadcaster entries). Reasons:
    // 1. These L1 entries don't affect L2 chain state correctness after a revert.
    // 2. They may be useful for debugging or auditing purposes.
    // 3. Deleting them adds complexity and potential for errors.

    if needs_checkpoint_cleanup {
        if !dry_run {
            println!(
                "Cleaning up checkpoints from epoch {first_epoch_to_clean} to {latest_checkpoint_epoch}"
            );

            // Bulk delete checkpoints
            checkpoints_to_delete = db
                .core
                .checkpoint_db()
                .del_checkpoints_from_epoch(first_epoch_to_clean)
                .internal_error("Failed to delete checkpoints")?;

            println!("Deleted checkpoints at epochs: {checkpoints_to_delete:?}");
        } else {
            // In dry run, collect what would be deleted
            for epoch in first_epoch_to_clean..=latest_checkpoint_epoch {
                checkpoints_to_delete.push(epoch);
            }
        }
    }

    if needs_epoch_summary_cleanup {
        let last_epoch = last_summarized_epoch.unwrap();
        if !dry_run {
            println!(
                "Cleaning up epoch summaries from epoch {first_epoch_to_clean} to {last_epoch}"
            );

            // Bulk delete epoch summaries
            epoch_summaries_to_delete = db
                .core
                .checkpoint_db()
                .del_epoch_summaries_from_epoch(first_epoch_to_clean)
                .internal_error("Failed to delete epoch summaries")?;
            println!("Deleted epoch summaries at epochs: {epoch_summaries_to_delete:?}");
        } else {
            // In dry run, collect what would be deleted
            for epoch in first_epoch_to_clean..=last_epoch {
                epoch_summaries_to_delete.push(epoch);
            }
        }
    }

    if dry_run {
        // Print dry run summary
        let has_changes = !write_batches_to_delete.is_empty()
            || !blocks_to_mark_unchecked.is_empty()
            || !checkpoints_to_delete.is_empty()
            || !epoch_summaries_to_delete.is_empty();

        println!();
        println!("========================================");
        println!("DRY RUN SUMMARY - No changes were made");
        println!("========================================");
        println!();

        if !has_changes {
            println!("No changes would be made.");
            println!();
            println!("This could be because:");
            println!("  - Target slot is at or near the latest slot");
            println!("  - All blocks after target are already in the expected state");
        } else {
            println!("This command would make the following changes:");
            println!();

            // Write batches
            if !write_batches_to_delete.is_empty() {
                let first_slot = write_batches_to_delete
                    .first()
                    .map(|(s, _)| *s)
                    .unwrap_or(0);
                let last_slot = write_batches_to_delete.last().map(|(s, _)| *s).unwrap_or(0);
                println!(
                    "Write batches to delete ({} total):",
                    write_batches_to_delete.len()
                );
                if first_slot == last_slot {
                    println!("  - Slot {first_slot}");
                } else {
                    println!("  - Slots {first_slot} to {last_slot} (inclusive)");
                }
                println!();
            }

            // Blocks marked as Unchecked
            if !blocks_to_mark_unchecked.is_empty() {
                println!(
                    "Blocks to mark as Unchecked ({} total):",
                    blocks_to_mark_unchecked.len()
                );
                for block_id in &blocks_to_mark_unchecked {
                    println!("  - {block_id:?}");
                }
                println!();
            }

            // Blocks to delete (only if -d flag is set)
            if args.delete_blocks && !blocks_to_delete.is_empty() {
                println!("Block data to delete ({} total):", blocks_to_delete.len());
                for block_id in &blocks_to_delete {
                    println!("  - {block_id:?}");
                }
                println!();
            }

            // Checkpoints and summaries
            if !checkpoints_to_delete.is_empty() {
                println!(
                    "Checkpoints to delete ({} total):",
                    checkpoints_to_delete.len()
                );
                println!("  - Epochs: {checkpoints_to_delete:?}");
                println!();
            }

            if !epoch_summaries_to_delete.is_empty() {
                println!(
                    "Epoch summaries to delete ({} total):",
                    epoch_summaries_to_delete.len()
                );
                println!("  - Epochs: {epoch_summaries_to_delete:?}");
                println!();
            }
        }

        println!("To execute this operation, run with --force/-f flag.");
    } else {
        println!();
        println!("Revert chainstate completed");
    }

    Ok(())
}
