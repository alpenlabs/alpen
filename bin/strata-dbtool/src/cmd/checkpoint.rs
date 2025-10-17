use argh::FromArgs;
use strata_asm_types::ProtocolOperation;
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_db::{
    traits::{CheckpointDatabase, DatabaseBackend},
    types::CheckpointEntry,
};

use super::l1::{get_l1_block_id_at_height, get_l1_block_manifest, get_l1_chain_tip};
use crate::{
    cli::OutputFormat,
    output::{
        checkpoint::{CheckpointInfo, CheckpointsSummaryInfo, EpochInfo},
        output,
    },
};

/// Get the last epoch index from the database.
///
/// This finds the highest epoch index in the database.
pub(crate) fn get_last_epoch(db: &impl DatabaseBackend) -> Result<Option<u64>, DisplayedError> {
    db.checkpoint_db()
        .get_last_summarized_epoch()
        .internal_error("Failed to get last summarized epoch")
}

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
    /// start l1 height to query checkpoints from
    #[argh(positional)]
    pub(crate) height_from: u64,

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

/// Get a checkpoint entry at a specific index.
///
/// Returns `None` if no checkpoint exists at that index.
pub(crate) fn get_checkpoint_at_index(
    db: &impl DatabaseBackend,
    index: u64,
) -> Result<Option<CheckpointEntry>, DisplayedError> {
    let chkpt_db = db.checkpoint_db();
    chkpt_db
        .get_checkpoint(index)
        .internal_error(format!("Failed to get checkpoint at index {}", index))
}

/// Get latest checkpoint entry.
pub(crate) fn get_latest_checkpoint_entry(
    db: &impl DatabaseBackend,
) -> Result<CheckpointEntry, DisplayedError> {
    let chkpt_db = db.checkpoint_db();
    let last_idx = chkpt_db
        .get_last_checkpoint_idx()
        .internal_error("Failed to get last checkpoint index")?
        .expect("valid checkpoint index");

    let checkpoint_entry = get_checkpoint_at_index(db, last_idx)?.ok_or_else(|| {
        DisplayedError::InternalError("No checkpoint found".to_string(), Box::new(last_idx))
    })?;
    Ok(checkpoint_entry)
}

/// Get the range of checkpoint indices (0 to latest).
///
/// Returns `None` if no checkpoints exist, otherwise returns `Some((0, latest_idx))`.
pub(crate) fn get_checkpoint_index_range(
    db: &impl DatabaseBackend,
) -> Result<Option<(u64, u64)>, DisplayedError> {
    let chkpt_db = db.checkpoint_db();
    if let Some(last_idx) = chkpt_db
        .get_last_checkpoint_idx()
        .internal_error("Failed to get last checkpoint index")?
    {
        Ok(Some((0, last_idx)))
    } else {
        Ok(None)
    }
}

/// Get checkpoint details by index.
pub(crate) fn get_checkpoint(
    db: &impl DatabaseBackend,
    args: GetCheckpointArgs,
) -> Result<(), DisplayedError> {
    let checkpoint_idx = args.checkpoint_index;
    let entry = get_checkpoint_at_index(db, checkpoint_idx)?.ok_or_else(|| {
        DisplayedError::UserError(
            "No checkpoint found at index".to_string(),
            Box::new(checkpoint_idx),
        )
    })?;

    // Create the output data structure
    let checkpoint_info = CheckpointInfo {
        checkpoint_index: checkpoint_idx,
        checkpoint: &entry.checkpoint,
        confirmation_status: &entry.confirmation_status,
        proving_status: &entry.proving_status,
    };

    // Use the output utility
    output(&checkpoint_info, args.output_format)
}

/// Get summary of all checkpoints.
pub(crate) fn get_checkpoints_summary(
    db: &impl DatabaseBackend,
    args: GetCheckpointsSummaryArgs,
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

    let mut found_checkpoints = 0;
    let mut unexpected_checkpoints = Vec::new();

    for l1_height in args.height_from..=l1_tip_height {
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

    // Convert unexpected checkpoints to the expected format
    let unexpected_checkpoints_info: Vec<crate::output::checkpoint::UnexpectedCheckpointInfo> =
        unexpected_checkpoints
            .into_iter()
            .map(|(checkpoint_index, l1_height)| {
                crate::output::checkpoint::UnexpectedCheckpointInfo {
                    checkpoint_index,
                    l1_height,
                }
            })
            .collect();

    // Create the output data structure
    let summary_info = CheckpointsSummaryInfo {
        expected_checkpoints_count,
        checkpoints_found_in_db,
        checkpoints_in_l1_blocks,
        unexpected_checkpoints: unexpected_checkpoints_info,
    };

    // Use the output utility
    output(&summary_info, args.output_format)
}

/// Get epoch summary at specified index.
pub(crate) fn get_epoch_summary(
    db: &impl DatabaseBackend,
    args: GetEpochSummaryArgs,
) -> Result<(), DisplayedError> {
    let chkpt_db = db.checkpoint_db();
    let epoch_idx = args.epoch_index;

    let epoch_commitments = chkpt_db
        .get_epoch_commitments_at(epoch_idx)
        .internal_error(format!(
            "Failed to get epoch commitments for epoch {epoch_idx}"
        ))?;

    if epoch_commitments.is_empty() {
        return Err(DisplayedError::UserError(
            "No epoch summary found for epoch".to_string(),
            Box::new(epoch_idx),
        ));
    }

    let epoch_summary = chkpt_db
        .get_epoch_summary(epoch_commitments[0])
        .internal_error(format!("Failed to get epoch summary for epoch {epoch_idx}",))?
        .ok_or_else(|| {
            DisplayedError::UserError(
                format!("No epoch summary found for epoch {epoch_idx}"),
                Box::new(epoch_idx),
            )
        })?;

    // Create the output data structure
    let epoch_info = EpochInfo {
        epoch_index: epoch_idx,
        epoch_summary: &epoch_summary,
    };

    // Use the output utility
    output(&epoch_info, args.output_format)
}
