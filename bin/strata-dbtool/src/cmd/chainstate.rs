use argh::FromArgs;
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_consensus_logic::chain_worker_context::conv_blkid_to_slot_wb_id;
use strata_db_types::{
    chainstate::ChainstateDatabase,
    traits::{
        BlockStatus, CheckpointDatabase, ClientStateDatabase, DatabaseBackend, L2BlockDatabase,
    },
};
use strata_ol_chainstate_types::WriteBatch;
use strata_primitives::{l1::L1BlockCommitment, l2::L2BlockId};

use super::{
    checkpoint::get_latest_checkpoint_entry,
    l1::get_l1_chain_tip,
    l2::{get_chain_tip_block_id, get_chain_tip_slot, get_l2_block_slot_and_epoch},
};
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

    /// force execution (without this flag, only a dry run is performed)
    #[argh(switch, short = 'f')]
    pub(crate) force: bool,
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

/// Get the write batch for the chain tip L2 block.
///
/// This gets the write batch associated with the highest slot block in the database.
pub(crate) fn get_latest_l2_write_batch(
    db: &impl DatabaseBackend,
) -> Result<WriteBatch, DisplayedError> {
    let block_id = get_chain_tip_block_id(db)?;
    get_l2_write_batch(db, block_id)?.ok_or_else(|| {
        DisplayedError::InternalError("L2 write batch not found".to_string(), Box::new(block_id))
    })
}

/// Deletes ClientState entries from a given L1 block onwards
/// Returns the list of L1BlockCommitments that were deleted (or would be deleted in dry run)
fn delete_client_states_from(
    db: &impl DatabaseBackend,
    from_l1_block: L1BlockCommitment,
    l1_tip_height: u64,
    dry_run: bool,
) -> Result<Vec<L1BlockCommitment>, DisplayedError> {
    let client_state_db = db.client_state_db();

    // Calculate max possible entries from from_block to L1 tip
    let from_height = from_l1_block.height_u64();
    let max_count = if l1_tip_height >= from_height {
        (l1_tip_height - from_height + 1) as usize
    } else {
        // No entries to fetch if from_height is beyond L1 tip
        return Ok(Vec::new());
    };

    // Fetch all ClientState entries in one call
    let updates = client_state_db
        .get_client_updates_from(from_l1_block, max_count)
        .internal_error("Failed to get client state updates")?;

    let mut entries = Vec::new();

    // Collect and optionally delete each ClientState update
    for (l1_block, _) in updates {
        entries.push(l1_block);

        if !dry_run {
            client_state_db
                .del_client_update(l1_block)
                .internal_error("Failed to delete client state update")?;
        }
    }

    Ok(entries)
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
    let (block_slot, block_epoch) =
        get_l2_block_slot_and_epoch(db, block_id)?.ok_or_else(|| {
            DisplayedError::UserError("L2 block with id not found".to_string(), Box::new(block_id))
        })?;

    let prev_epoch = top_level_state.prev_epoch();
    let finalized_epoch = top_level_state.finalized_epoch();
    let l1_view_state = top_level_state.l1_view();

    // Create the output data structure
    let chainstate_info = ChainstateInfo {
        block_id: &block_id,
        current_slot: block_slot,
        current_epoch: block_epoch,
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
    db: &impl DatabaseBackend,
    args: RevertChainstateArgs,
) -> Result<(), DisplayedError> {
    let target_block_id = parse_l2_block_id(&args.block_id)?;
    let (target_slot, target_epoch) = get_l2_block_slot_and_epoch(db, target_block_id)?
        .ok_or_else(|| {
            DisplayedError::UserError(
                "L2 block with id not found".to_string(),
                Box::new(target_block_id),
            )
        })?;

    // Dry run mode by default - user must explicitly force execution
    let dry_run = !args.force;

    // Get the chain tip slot
    let chain_tip_slot = get_chain_tip_slot(db)?;

    // Get chain tip write batch to check finalized epoch constraints
    let chain_tip_write_batch = get_latest_l2_write_batch(db)?;
    let chain_tip_top_level_state = chain_tip_write_batch.new_toplevel_state();
    let finalized_slot = chain_tip_top_level_state.finalized_epoch().last_slot();

    if target_slot < finalized_slot {
        return Err(DisplayedError::UserError(
            "Target block is inside finalized epoch".to_string(),
            Box::new(target_block_id),
        ));
    }

    // Check if target block is inside checkpointed epoch
    let latest_checkpoint_entry = get_latest_checkpoint_entry(db)?;
    let latest_checkpoint_epoch = latest_checkpoint_entry.checkpoint.batch_info().epoch;
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

    // Get the target block's write batch to find the L1 safe block at that point
    let target_write_batch = get_l2_write_batch(db, target_block_id)?.ok_or_else(|| {
        DisplayedError::UserError(
            "Target L2 write batch not found".to_string(),
            Box::new(target_block_id),
        )
    })?;
    let target_top_level_state = target_write_batch.new_toplevel_state();
    let target_l1_safe_block = target_top_level_state.l1_view().get_safe_block();
    let target_slot_is_terminal = target_top_level_state.is_epoch_finishing();

    println!("Chainstate chain tip slot {chain_tip_slot}");
    println!("Chainstate finalized slot {finalized_slot}");
    println!("Latest checkpointed slot {checkpoint_last_slot}");
    println!("Revert chainstate target slot {target_slot}");
    println!("Target slot is epoch finishing: {target_slot_is_terminal}");
    println!(
        "Target L1 safe block {}@{}",
        target_l1_safe_block.height_u64(),
        target_l1_safe_block.blkid()
    );
    println!();

    // Check if there are any blocks to revert
    if target_slot >= chain_tip_slot {
        if dry_run {
            println!("========================================");
            println!("DRY RUN SUMMARY - No changes were made");
            println!("========================================");
            println!();
            println!("No changes would be made.");
            println!();
            println!(
                "Target slot ({}) is at or after the chain tip slot ({}).",
                target_slot, chain_tip_slot
            );
            println!("Nothing to revert.");
        } else {
            println!();
            println!(
                "NOTE: Target slot ({}) is at or after the chain tip slot ({}).",
                target_slot, chain_tip_slot
            );
            println!("Nothing to revert.");
        }
        return Ok(());
    }

    // Collect statistics and optionally execute deletions
    if !dry_run {
        println!("Executing revert operation...");
        println!();
    }

    let mut write_batches_to_delete = Vec::new();
    let mut blocks_to_mark_unchecked = Vec::new();
    let mut blocks_to_delete = Vec::new();

    for slot in target_slot + 1..=chain_tip_slot {
        let l2_block_ids = db.l2_db().get_blocks_at_height(slot).unwrap_or_default();
        for block_id in l2_block_ids.iter() {
            // Convert block ID to write batch ID
            let write_batch_id = conv_blkid_to_slot_wb_id(*block_id);

            // Check if write batch exists
            let write_batch_exists = db
                .chain_state_db()
                .get_write_batch(write_batch_id)
                .internal_error("Failed to check write batch existence")?
                .is_some();

            if write_batch_exists {
                // Collect statistics
                write_batches_to_delete.push(write_batch_id);
                blocks_to_mark_unchecked.push(*block_id);
                if args.delete_blocks {
                    blocks_to_delete.push(*block_id);
                }

                // Execute deletion if not dry run
                if !dry_run {
                    println!("Revert chainstate deleting write batch {block_id:?} {slot}");
                    db.chain_state_db()
                        .del_write_batch(write_batch_id)
                        .internal_error(format!(
                            "Failed to delete write batch for block {}",
                            *block_id
                        ))?;

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
                            .internal_error(format!(
                                "Failed to delete block with id {}",
                                *block_id
                            ))?;
                    }
                }
            } else if !dry_run {
                println!("Revert chainstate no write batch found {block_id:?} {slot}");
            }
        }
    }

    // Collect checkpoint and client state statistics
    let mut client_state_entries_to_delete = Vec::new();
    let mut checkpoints_to_delete = Vec::new();
    let mut epoch_summaries_to_delete = Vec::new();

    // Determine first epoch to clean up
    // - If target_slot is terminal: target_epoch is complete, start cleaning from next epoch
    // - If target_slot is not terminal: target_epoch is incomplete, include it in cleanup
    let first_epoch_to_clean = if target_slot_is_terminal {
        target_epoch + 1
    } else {
        target_epoch
    };

    // Check if there are checkpoints to clean
    let needs_checkpoint_cleanup = first_epoch_to_clean <= (latest_checkpoint_epoch as u64);

    // Check if there are epoch summaries to clean (may exist beyond latest checkpoint)
    let last_summarized_epoch = db
        .checkpoint_db()
        .get_last_summarized_epoch()
        .internal_error("Failed to get last summarized epoch")?;

    let needs_epoch_summary_cleanup = last_summarized_epoch
        .map(|last_epoch| first_epoch_to_clean <= last_epoch)
        .unwrap_or(false);

    if needs_checkpoint_cleanup {
        // Process ClientState entries AFTER the target L1 safe block
        let next_l1_height = target_l1_safe_block.height_u64() + 1;
        let next_l1_block = L1BlockCommitment::from_height_u64(next_l1_height, Default::default())
            .ok_or_else(|| {
                DisplayedError::InternalError(
                    "Failed to create next L1 block commitment".to_string(),
                    Box::new(next_l1_height),
                )
            })?;

        // Get L1 tip for informational purposes
        let (l1_tip_height, l1_tip_block_id) = get_l1_chain_tip(db)?;

        // Collect and optionally delete ClientState entries
        if !dry_run {
            println!(
                "Revert chainstate deleting ClientState entries from L1 height {} to {} (L1 tip)",
                next_l1_height, l1_tip_height
            );
        }

        match delete_client_states_from(db, next_l1_block, l1_tip_height, dry_run) {
            Ok(entries) => {
                client_state_entries_to_delete = entries;
                if !dry_run {
                    println!(
                        "Deleted {} ClientState entries",
                        client_state_entries_to_delete.len()
                    );
                    println!(
                        "  (L1 range: height {} to {}, L1 tip: {})",
                        next_l1_height, l1_tip_height, l1_tip_block_id
                    );
                }
            }
            Err(e) => {
                if !dry_run {
                    println!("Warning: Failed to delete ClientState entries: {}", e);
                }
                return Err(e);
            }
        }

        // Collect checkpoint statistics - we know the range from first_epoch_to_clean to
        // latest_checkpoint_epoch
        for epoch in first_epoch_to_clean..=(latest_checkpoint_epoch as u64) {
            checkpoints_to_delete.push(epoch);
        }

        // Note: We intentionally do NOT delete L1 related stuff ( writer entries such as
        // intent/payload, broadcast entries or ASM related stuff). Reason is twofold:
        // 1. These L1 entries don't affect L2 chain state correctness after a revert.
        // 2. The L1 transactions may already be on Bitcoin, so keeping the records is appropriate.

        // Execute checkpoint deletion if not dry run
        if !dry_run {
            println!(
                "Revert chainstate cleaning up checkpoints from epoch {first_epoch_to_clean} to {latest_checkpoint_epoch}"
            );

            // Use bulk deletion methods for efficiency
            let deleted_checkpoints = db
                .checkpoint_db()
                .del_checkpoints_from_epoch(first_epoch_to_clean)
                .internal_error("Failed to delete checkpoints")?;

            println!("Deleted checkpoints at epochs: {:?}", deleted_checkpoints);
        }
    } else if !dry_run {
        println!("No checkpoint cleanup needed - target slot preserves all checkpointed epochs");
    }

    // Handle epoch summary cleanup separately (may extend beyond checkpoints)
    if needs_epoch_summary_cleanup {
        let last_epoch = last_summarized_epoch.unwrap();

        // Collect epoch summary statistics
        for epoch in first_epoch_to_clean..=last_epoch {
            epoch_summaries_to_delete.push(epoch);
        }

        // Execute epoch summary deletion if not dry run
        if !dry_run {
            println!(
                "Revert chainstate cleaning up epoch summaries from epoch {first_epoch_to_clean} to {last_epoch}"
            );

            let deleted_summaries = db
                .checkpoint_db()
                .del_epoch_summaries_from_epoch(first_epoch_to_clean)
                .internal_error("Failed to delete epoch summaries")?;

            println!("Deleted epoch summaries at epochs: {:?}", deleted_summaries);
        }
    } else if !dry_run {
        println!("No epoch summary cleanup needed");
    }

    println!();
    if dry_run {
        // Check if no changes would be made
        let has_changes = !write_batches_to_delete.is_empty()
            || (!client_state_entries_to_delete.is_empty()
                || !checkpoints_to_delete.is_empty()
                || !epoch_summaries_to_delete.is_empty());

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
                println!(
                    "Write Batches to delete ({} total):",
                    write_batches_to_delete.len()
                );
                for write_batch_id in &write_batches_to_delete {
                    println!("  - Write batch {:?}", write_batch_id);
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
                    println!("  - {:?}", block_id);
                }
                println!();
            }

            // Blocks to delete (only if -d flag is set)
            if args.delete_blocks && !blocks_to_delete.is_empty() {
                println!("Block data to delete ({} total):", blocks_to_delete.len());
                for block_id in &blocks_to_delete {
                    println!("  - {:?}", block_id);
                }
                println!();
            }

            // ClientState entries (only if checkpoint cleanup is needed)
            if needs_checkpoint_cleanup && !client_state_entries_to_delete.is_empty() {
                let first_height = client_state_entries_to_delete
                    .first()
                    .map(|b| b.height_u64());
                let last_height = client_state_entries_to_delete
                    .last()
                    .map(|b| b.height_u64());

                if let (Some(first), Some(last)) = (first_height, last_height) {
                    println!(
                        "ClientState entries to delete ({} total, L1 height {} to {}):",
                        client_state_entries_to_delete.len(),
                        first,
                        last
                    );
                } else {
                    println!(
                        "ClientState entries to delete ({} total):",
                        client_state_entries_to_delete.len()
                    );
                }

                for l1_block in &client_state_entries_to_delete {
                    println!(
                        "  - L1 block {} at height {}",
                        l1_block.blkid(),
                        l1_block.height_u64()
                    );
                }
                println!();
            }

            if !checkpoints_to_delete.is_empty() {
                println!(
                    "Checkpoints to delete at epochs: {:?}",
                    checkpoints_to_delete
                );
                println!();
            }

            if !epoch_summaries_to_delete.is_empty() {
                println!(
                    "Epoch summaries to delete at epochs: {:?}",
                    epoch_summaries_to_delete
                );
                println!();
            }

            // Only show finalized blocks note if target is inside or near finalized epoch
            if target_slot <= finalized_slot || target_slot < finalized_slot + 10 {
                println!(
                    "NOTE: Finalized blocks (slot < {}) cannot be reverted.",
                    finalized_slot
                );
                println!();
            }

            println!("If you understand the consequences and still want to revert chainstate,");
            println!("re-run with the --force (-f) flag.");
        }
    } else {
        println!("Revert chainstate completed");
    }
    Ok(())
}
