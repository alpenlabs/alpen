use argh::FromArgs;
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_consensus_logic::chain_worker_context::conv_blkid_to_slot_wb_id;
use strata_db::{
    chainstate::ChainstateDatabase,
    traits::{
        BlockStatus, CheckpointDatabase, ClientStateDatabase, DatabaseBackend, L2BlockDatabase,
    },
};
use strata_ol_chainstate_types::WriteBatch;
use strata_primitives::{l1::L1BlockCommitment, l2::L2BlockId};

use super::{
    checkpoint::get_latest_checkpoint_entry,
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
fn delete_client_states_from(
    db: &impl DatabaseBackend,
    from_l1_block: L1BlockCommitment,
) -> Result<usize, DisplayedError> {
    let client_state_db = db.client_state_db();
    let mut deleted_count = 0;
    let mut current_block = from_l1_block;

    // Get all ClientState updates from the specified L1 block onwards
    // We fetch in batches to avoid loading too many at oncloadinge
    const BATCH_SIZE: usize = 100;
    loop {
        let updates = client_state_db
            .get_client_updates_from(current_block, BATCH_SIZE)
            .internal_error("Failed to get client state updates")?;

        let batch_size = updates.len();

        // Delete each ClientState update and track the last one for next iteration
        for (l1_block, _) in updates {
            client_state_db
                .del_client_update(l1_block)
                .internal_error("Failed to delete client state update")?;
            deleted_count += 1;
            current_block = l1_block;
        }

        // If we got fewer than BATCH_SIZE, we've reached the end
        if batch_size < BATCH_SIZE {
            break;
        }
    }

    Ok(deleted_count)
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

    // Now delete write batches and optionally blocks
    for slot in target_slot + 1..=chain_tip_slot {
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

    // Determine first epoch to clean up
    // - If target_slot is terminal: target_epoch is complete, start cleaning from next epoch
    // - If target_slot is not terminal: target_epoch is incomplete, include it in cleanup
    let first_epoch_to_clean = if target_slot_is_terminal {
        target_epoch + 1
    } else {
        target_epoch
    };

    // Only perform cleanup if there are epochs to clean
    let needs_cleanup = first_epoch_to_clean <= latest_checkpoint_epoch;

    if needs_cleanup {
        // Delete ClientState entries AFTER the target L1 safe block
        let next_l1_height = target_l1_safe_block.height_u64() + 1;
        let next_l1_block = L1BlockCommitment::from_height_u64(next_l1_height, Default::default())
            .ok_or_else(|| {
                DisplayedError::InternalError(
                    "Failed to create next L1 block commitment".to_string(),
                    Box::new(next_l1_height),
                )
            })?;

        println!(
            "Revert chainstate deleting ClientState entries from L1 height {} onwards",
            next_l1_height
        );
        match delete_client_states_from(db, next_l1_block) {
            Ok(count) => {
                println!("Deleted {} ClientState entries", count);
            }
            Err(e) => {
                println!("Warning: Failed to delete ClientState entries: {}", e);
            }
        }

        println!(
            "Cleaning up checkpoints and epoch summaries from epoch {first_epoch_to_clean} to {latest_checkpoint_epoch}"
        );

        // Note: We intentionally do NOT delete L1 related stuff ( writer entries such as
        // intent/payload, broadcast entries or ASM related stuff). Reason is twofold:
        // 1. These L1 entries don't affect L2 chain state correctness after a revert.
        // 2. The L1 transactions may already be on Bitcoin, so keeping the records is appropriate.

        // Bulk delete checkpoints and epoch summaries
        let deleted_checkpoints = db
            .checkpoint_db()
            .del_checkpoints_from_epoch(first_epoch_to_clean)
            .internal_error("Failed to delete checkpoints")?;

        let deleted_summaries = db
            .checkpoint_db()
            .del_epoch_summaries_from_epoch(first_epoch_to_clean)
            .internal_error("Failed to delete epoch summaries")?;

        println!("Deleted checkpoints at epochs: {:?}", deleted_checkpoints);
        println!("Deleted epoch summaries at epochs: {:?}", deleted_summaries);
    } else {
        println!("No cleanup needed - target slot preserves all checkpointed epochs");
    }

    println!("Revert chainstate completed");
    Ok(())
}
