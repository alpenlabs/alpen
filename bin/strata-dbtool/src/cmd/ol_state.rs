use argh::FromArgs;
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_db_types::traits::{
    BlockStatus, DatabaseBackend, OLBlockDatabase, OLCheckpointDatabase, OLStateDatabase,
};
use strata_identifiers::{Epoch, EpochCommitment, OLBlockCommitment, OLBlockId, Slot};
use strata_ledger_types::IStateAccessor;

use super::client_state::get_declared_final_epoch;
use crate::{
    cli::OutputFormat,
    output::{ol_state::OLStateInfo, output},
    utils::block_id::parse_ol_block_id,
};

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "get-ol-state")]
/// Get OL state at specified block
pub(crate) struct GetOLStateArgs {
    /// OL block id
    #[argh(positional)]
    pub(crate) block_id: String,

    /// output format: "porcelain" (default) or "json"
    #[argh(option, short = 'o', default = "OutputFormat::Porcelain")]
    pub(crate) output_format: OutputFormat,
}

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "revert-ol-state")]
/// Revert OL state to specified block
pub(crate) struct RevertOLStateArgs {
    /// target OL block id
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

/// Get OL state at specified block.
pub(crate) fn get_ol_state(
    db: &impl DatabaseBackend,
    args: GetOLStateArgs,
) -> Result<(), DisplayedError> {
    let block_id = parse_ol_block_id(&args.block_id)?;
    let (block_slot, block_epoch) =
        get_ol_block_slot_and_epoch(db, block_id)?.ok_or_else(|| {
            DisplayedError::UserError("OL block with id not found".to_string(), Box::new(block_id))
        })?;

    let commitment = OLBlockCommitment::new(block_slot, block_id);
    let top_level_state = db
        .ol_state_db()
        .get_toplevel_ol_state(commitment)
        .internal_error("Failed to get OL state")?
        .ok_or_else(|| {
            DisplayedError::UserError(
                "OL state not found for block".to_string(),
                Box::new(commitment),
            )
        })?;

    let ol_block = db
        .ol_block_db()
        .get_block_data(block_id)
        .internal_error("Failed to read OL block data")?
        .ok_or_else(|| {
            DisplayedError::UserError("OL block with id not found".to_string(), Box::new(block_id))
        })?;

    // OL state currently exposes ASM-recorded epoch for previous-epoch view.
    let recorded_epoch = top_level_state.asm_recorded_epoch();
    // Finalized epoch should come from client-state declared final epoch (L1-confirmed).
    let finalized_epoch = get_declared_final_epoch(db)?.unwrap_or_else(EpochCommitment::null);
    let l1_safe_block_height = top_level_state.last_l1_height();
    let ol_state_info = OLStateInfo {
        block_id: &block_id,
        current_slot: block_slot,
        current_epoch: block_epoch,
        is_epoch_finishing: ol_block.body().l1_update().is_some(),
        previous_epoch: recorded_epoch,
        finalized_epoch: &finalized_epoch,
        l1_next_expected_height: l1_safe_block_height.saturating_add(1),
        l1_safe_block_height,
        l1_safe_block_blkid: top_level_state.last_l1_blkid(),
    };

    output(&ol_state_info, args.output_format)
}

/// Revert OL state to specified block.
pub(crate) fn revert_ol_state(
    db: &impl DatabaseBackend,
    args: RevertOLStateArgs,
) -> Result<(), DisplayedError> {
    let target_block_id = parse_ol_block_id(&args.block_id)?;
    let (target_slot, target_epoch) = get_ol_block_slot_and_epoch(db, target_block_id)?
        .ok_or_else(|| {
            DisplayedError::UserError(
                "OL block with id not found".to_string(),
                Box::new(target_block_id),
            )
        })?;

    let target_block = db
        .ol_block_db()
        .get_block_data(target_block_id)
        .internal_error("Failed to read target OL block")?
        .ok_or_else(|| {
            DisplayedError::UserError(
                "OL block with id not found".to_string(),
                Box::new(target_block_id),
            )
        })?;
    let target_slot_is_terminal = target_block.body().l1_update().is_some();

    let dry_run = !args.force;

    let chain_tip_slot = db
        .ol_block_db()
        .get_tip_slot()
        .internal_error("Failed to get OL tip slot")?;

    // No-op: target is already at/after current tip.
    if target_slot >= chain_tip_slot {
        println!("No changes would be made.");
        println!(
            "Target slot ({}) is at or after the chain tip slot ({}).",
            target_slot, chain_tip_slot
        );
        return Ok(());
    }

    let finalized_epoch = get_declared_final_epoch(db)?.unwrap_or_else(EpochCommitment::null);
    let finalized_slot = finalized_epoch.last_slot();
    if target_slot < finalized_slot {
        return Err(DisplayedError::UserError(
            "Target block is inside finalized epoch".to_string(),
            Box::new(target_block_id),
        ));
    }

    let checkpoint_last_slot = get_latest_checkpoint_last_slot(db)?;
    if !args.revert_checkpointed_blocks && target_slot < checkpoint_last_slot {
        return Err(DisplayedError::UserError(
            "Target block is inside checkpointed epoch".to_string(),
            Box::new(target_block_id),
        ));
    }

    println!("OL state chain tip slot {chain_tip_slot}");
    println!("OL state finalized slot {finalized_slot}");
    println!("Latest checkpointed slot {checkpoint_last_slot}");
    println!("Revert OL state target slot {target_slot}");
    println!("Target slot is epoch finishing: {target_slot_is_terminal}");
    println!();

    let mut commitments_to_delete = Vec::new();
    let mut blocks_to_mark_unchecked = Vec::new();
    let mut blocks_to_delete = Vec::new();

    for slot in target_slot + 1..=chain_tip_slot {
        let block_ids = db
            .ol_block_db()
            .get_blocks_at_height(slot)
            .internal_error(format!("Failed to get OL blocks at slot {slot}"))?;

        for block_id in block_ids {
            let commitment = OLBlockCommitment::new(slot, block_id);
            let has_state = db
                .ol_state_db()
                .get_toplevel_ol_state(commitment)
                .internal_error("Failed to check OL state existence")?
                .is_some();

            if has_state {
                commitments_to_delete.push(commitment);
                blocks_to_mark_unchecked.push(block_id);
                if args.delete_blocks {
                    blocks_to_delete.push(block_id);
                }

                if !dry_run {
                    db.ol_state_db()
                        .del_ol_write_batch(commitment)
                        .internal_error("Failed to delete OL write batch")?;
                    db.ol_state_db()
                        .del_toplevel_ol_state(commitment)
                        .internal_error("Failed to delete OL state")?;
                    db.ol_block_db()
                        .set_block_status(block_id, BlockStatus::Unchecked)
                        .internal_error("Failed to set OL block status")?;

                    if args.delete_blocks {
                        db.ol_block_db()
                            .del_block_data(block_id)
                            .internal_error("Failed to delete OL block")?;
                    }
                }
            }
        }
    }

    let first_epoch_to_clean = if target_slot_is_terminal {
        target_epoch + 1
    } else {
        target_epoch
    };

    let mut checkpoints_to_delete = Vec::new();
    let mut epoch_summaries_to_delete = Vec::new();

    if let Some(last_epoch) = db
        .ol_checkpoint_db()
        .get_last_checkpoint_epoch()
        .internal_error("Failed to get last checkpoint epoch")?
    {
        for epoch in first_epoch_to_clean..=last_epoch {
            checkpoints_to_delete.push(epoch);
            epoch_summaries_to_delete.push(epoch);
        }

        if !dry_run {
            let _ = db
                .ol_checkpoint_db()
                .del_checkpoints_from_epoch(first_epoch_to_clean)
                .internal_error("Failed to delete OL checkpoints")?;
            let _ = db
                .ol_checkpoint_db()
                .del_epoch_summaries_from_epoch(u64::from(first_epoch_to_clean))
                .internal_error("Failed to delete OL epoch summaries")?;
        }
    }

    let mode = if dry_run { "DRY RUN" } else { "EXECUTED" };
    println!("========================================");
    println!("{mode} SUMMARY");
    println!("========================================");
    println!(
        "OL states/write batches to delete: {}",
        commitments_to_delete.len()
    );
    println!(
        "Blocks to mark unchecked: {}",
        blocks_to_mark_unchecked.len()
    );
    println!("Blocks to delete: {}", blocks_to_delete.len());
    println!("Checkpoints to delete: {}", checkpoints_to_delete.len());
    println!(
        "Epoch summaries to delete: {}",
        epoch_summaries_to_delete.len()
    );

    if dry_run {
        println!();
        println!("Use --force to execute these changes.");
    }

    Ok(())
}

fn get_ol_block_slot_and_epoch(
    db: &impl DatabaseBackend,
    block_id: OLBlockId,
) -> Result<Option<(Slot, Epoch)>, DisplayedError> {
    let Some(block) = db
        .ol_block_db()
        .get_block_data(block_id)
        .internal_error("Failed to read OL block data")?
    else {
        return Ok(None);
    };

    Ok(Some((block.header().slot(), block.header().epoch())))
}

fn get_latest_checkpoint_last_slot(db: &impl DatabaseBackend) -> Result<Slot, DisplayedError> {
    let Some(last_epoch) = db
        .ol_checkpoint_db()
        .get_last_checkpoint_epoch()
        .internal_error("Failed to get last checkpoint epoch")?
    else {
        return Ok(0);
    };

    let checkpoint = db
        .ol_checkpoint_db()
        .get_checkpoint(last_epoch)
        .internal_error("Failed to get OL checkpoint")?
        .ok_or_else(|| {
            DisplayedError::InternalError(
                "Last checkpoint epoch exists but checkpoint entry is missing".to_string(),
                Box::new(last_epoch),
            )
        })?;

    Ok(checkpoint.checkpoint.new_tip().l2_commitment().slot())
}
