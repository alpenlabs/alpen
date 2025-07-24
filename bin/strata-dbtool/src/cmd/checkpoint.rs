use argh::FromArgs;
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_db::{
    traits::{CheckpointDatabase, Database},
    types::CheckpointEntry,
};
use strata_primitives::l1::ProtocolOperation;

use super::l1::{get_l1_block_id_at_height, get_l1_block_manifest, get_l1_chain_tip};
use crate::{cli::OutputFormat, cmd::client_state::get_latest_client_state_update};

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "get-checkpoint")]
/// Get checkpoint
pub(crate) struct GetCheckpointArgs {
    /// checkpoint index
    #[argh(positional)]
    pub(crate) checkpoint_index: u64,

    /// output format: "porcelain" (default) or "json"
    #[argh(option, short = 'o', default = "OutputFormat::Porcelain")]
    pub(crate) output_format: OutputFormat,
}

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "get-checkpoints-summary")]
/// Get checkpoints summary
pub(crate) struct GetCheckpointsSummaryArgs {
    /// output format: "porcelain" (default) or "json"
    #[argh(option, short = 'o', default = "OutputFormat::Porcelain")]
    pub(crate) output_format: OutputFormat,
}

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "get-epoch-summary")]
/// Get epoch summary
pub(crate) struct GetEpochSummaryArgs {
    /// epoch index
    #[argh(positional)]
    pub(crate) epoch_index: u64,

    /// output format: "porcelain" (default) or "json"
    #[argh(option, short = 'o', default = "OutputFormat::Porcelain")]
    pub(crate) output_format: OutputFormat,
}

/// Get checkpoint details by index.
pub(crate) fn get_checkpoint(
    db: &impl Database,
    args: GetCheckpointArgs,
) -> Result<(), DisplayedError> {
    let chkpt_db = db.checkpoint_db();
    let checkpoint_idx = args.checkpoint_index;
    let entry = chkpt_db
        .get_checkpoint(checkpoint_idx)
        .internal_error(format!(
            "Failed to get checkpoint at index {checkpoint_idx}",
        ))?
        .ok_or_else(|| {
            DisplayedError::UserError(
                "No checkpoint found at index".to_string(),
                Box::new(checkpoint_idx),
            )
        })?;

    // Print in porcelain format
    println!("checkpoint.index {checkpoint_idx}");
    println!("checkpoint.proving_status {:?}", entry.proving_status);
    println!(
        "checkpoint.confirmation_status {:?}",
        entry.confirmation_status
    );

    let checkpoint = &entry.checkpoint;
    let batch_info = checkpoint.batch_info();
    println!("checkpoint.batch.epoch {}", batch_info.epoch());
    println!(
        "checkpoint.batch.l1_range.start.height {}",
        batch_info.l1_range.0.height()
    );
    println!(
        "checkpoint.batch.l1_range.start.blkid {:?}",
        batch_info.l1_range.0.blkid()
    );
    println!(
        "checkpoint.batch.l1_range.end.height {}",
        batch_info.l1_range.1.height()
    );
    println!(
        "checkpoint.batch.l1_range.end.blkid {:?}",
        batch_info.l1_range.1.blkid()
    );
    println!(
        "checkpoint.batch.l2_range.start.slot {}",
        batch_info.l2_range.0.slot()
    );
    println!(
        "checkpoint.batch.l2_range.start.blkid {:?}",
        batch_info.l2_range.0.blkid()
    );
    println!(
        "checkpoint.batch.l2_range.end.slot {}",
        batch_info.l2_range.1.slot()
    );
    println!(
        "checkpoint.batch.l2_range.end.blkid {:?}",
        batch_info.l2_range.1.blkid()
    );

    let batch_transition = checkpoint.batch_transition();
    println!(
        "checkpoint.batch_transition.chainstate.pre_root {:?}",
        batch_transition.chainstate_transition.pre_state_root
    );
    println!(
        "checkpoint.batch_transition.chainstate.post_root {:?}",
        batch_transition.chainstate_transition.post_state_root
    );
    println!(
        "checkpoint.batch_transition.tx_filter.pre_config_hash {:?}",
        batch_transition.tx_filters_transition.pre_config_hash
    );
    println!(
        "checkpoint.batch_transition.tx_filter.post_config_hash {:?}",
        batch_transition.tx_filters_transition.post_config_hash
    );

    Ok(())
}

/// Get latest checkpoint entry.
pub(crate) fn get_latest_checkpoint_entry(
    db: &impl Database,
) -> Result<CheckpointEntry, DisplayedError> {
    let chkpt_db = db.checkpoint_db();
    let last_idx = chkpt_db
        .get_last_checkpoint_idx()
        .internal_error("Failed to get last checkpoint index")?
        .expect("valid checkpoint index");

    let checkpoint_entry = chkpt_db
        .get_checkpoint(last_idx)
        .internal_error("Failed to get last checkpoint")?
        .ok_or_else(|| {
            DisplayedError::InternalError("No checkpoint found".to_string(), Box::new(last_idx))
        })?;

    Ok(checkpoint_entry)
}

/// Get summary of all checkpoints.
pub(crate) fn get_checkpoints_summary(
    db: &impl Database,
    _args: GetCheckpointsSummaryArgs,
) -> Result<(), DisplayedError> {
    let chkpt_db = db.checkpoint_db();
    let last_idx = chkpt_db
        .get_last_checkpoint_idx()
        .internal_error("Failed to get last checkpoint index")?
        .expect("valid checkpoint index");

    let expected_checkpoints_count = last_idx + 1;
    let mut checkpoint_commitments = Vec::new();
    for idx in 0..=last_idx {
        let entry = chkpt_db
            .get_checkpoint(idx)
            .internal_error(format!("Failed to get checkpoint at index {idx}"))?;

        if let Some(checkpoint_entry) = entry {
            checkpoint_commitments.push(checkpoint_entry.checkpoint.commitment().clone());
        }
    }
    let checkpoints_found_in_db = checkpoint_commitments.len() as u64;

    // Check if all checkpoints are present in L1 blocks
    // Use helper function to get L1 tip
    let (l1_tip_height, _) = get_l1_chain_tip(db)?;

    let (client_state_update, _) = get_latest_client_state_update(db, None)?;
    let (client_state, _) = client_state_update.into_parts();
    let horizon_l1_height = client_state.horizon_l1_height();

    let mut found_checkpoints = 0;
    let mut unexpected_checkpoints = Vec::new();

    for l1_height in horizon_l1_height..=l1_tip_height {
        // Use helper functions to get block ID and manifest
        let block_id = get_l1_block_id_at_height(db, l1_height)?;
        let Some(manifest) = get_l1_block_manifest(db, block_id)? else {
            // Skip this block if manifest is missing
            continue;
        };

        manifest
            .txs()
            .iter()
            .flat_map(|tx| tx.protocol_ops())
            .filter_map(|op| match op {
                ProtocolOperation::Checkpoint(signed_checkpoint) => Some((
                    signed_checkpoint.checkpoint().commitment(),
                    signed_checkpoint.checkpoint().batch_info().epoch(),
                )),
                _ => None,
            })
            .for_each(|(commitment, checkpoint_index)| {
                if !checkpoint_commitments.contains(commitment) {
                    unexpected_checkpoints.push((checkpoint_index, l1_height));
                } else {
                    found_checkpoints += 1;
                }
            });
    }

    let checkpoints_in_l1_blocks = found_checkpoints as u64;

    // Print in porcelain format
    println!("checkpoints_summary.expected_checkpoints_count {expected_checkpoints_count}");
    println!("checkpoints_summary.checkpoints_found_in_db {checkpoints_found_in_db}");
    println!("checkpoints_summary.checkpoints_in_l1_blocks {checkpoints_in_l1_blocks}");

    if !unexpected_checkpoints.is_empty() {
        println!(
            "checkpoints_summary.unexpected_checkpoints_count {}",
            unexpected_checkpoints.len()
        );
        for (checkpoint_index, l1_height) in unexpected_checkpoints {
            println!("checkpoints_summary.unexpected_checkpoint.index {checkpoint_index}");
            println!("checkpoints_summary.unexpected_checkpoint.l1_height {l1_height}");
        }
    }

    Ok(())
}

/// Get epoch summary at specified index.
pub(crate) fn get_epoch_summary(
    db: &impl Database,
    args: GetEpochSummaryArgs,
) -> Result<(), DisplayedError> {
    let chkpt_db = db.checkpoint_db();
    let epoch_idx = args.epoch_index;

    let epoch_commitments = chkpt_db
        .get_epoch_commitments_at(epoch_idx)
        .internal_error(format!("Failed to get epoch summary for epoch {epoch_idx}",))?;

    let epoch_summary = chkpt_db
        .get_epoch_summary(epoch_commitments[0])
        .internal_error(format!("Failed to get epoch summary for epoch {epoch_idx}"))?
        .ok_or_else(|| {
            DisplayedError::UserError(
                format!("No epoch summary found for epoch {epoch_idx}"),
                Box::new(epoch_idx),
            )
        })?;

    // Print in porcelain format
    println!("epoch_summary.epoch_index {epoch_idx}");
    println!(
        "epoch_summary.epoch_commitment.epoch {}",
        epoch_summary.get_epoch_commitment().epoch()
    );
    println!(
        "epoch_summary.epoch_commitment.last_slot {}",
        epoch_summary.get_epoch_commitment().last_slot()
    );
    println!(
        "epoch_summary.epoch_commitment.last_blkid {:?}",
        epoch_summary.get_epoch_commitment().last_blkid()
    );
    println!(
        "epoch_summary.new_l1.height {}",
        epoch_summary.new_l1().height()
    );
    println!(
        "epoch_summary.new_l1.blkid {:?}",
        epoch_summary.new_l1().blkid()
    );

    Ok(())
}
