use argh::FromArgs;
use strata_asm_logs::CheckpointUpdate;
use strata_cli_common::errors::{DisplayableError, DisplayedError};
#[expect(deprecated, reason = "legacy old code is retained for compatibility")]
use strata_db_types::{
    traits::{AsmDatabase, CheckpointDatabase, DatabaseBackend, L1Database},
    types::CheckpointEntry,
};
use strata_primitives::l1::L1BlockCommitment;

use crate::{
    cli::OutputFormat,
    output::{
        checkpoint::{CheckpointInfo, CheckpointsSummaryInfo, EpochInfo, UnexpectedCheckpointInfo},
        output,
    },
};

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

/// Get the last epoch index from the database.
///
/// This finds the highest epoch index in the database.
pub(crate) fn get_last_epoch(db: &impl DatabaseBackend) -> Result<Option<u64>, DisplayedError> {
    #[expect(deprecated, reason = "legacy old code is retained for compatibility")]
    db.checkpoint_db()
        .get_last_summarized_epoch()
        .internal_error("Failed to get last summarized epoch")
}

/// Count unique checkpoints found in ASM logs starting from a given L1 height.
///
/// This scans ASM states from the specified height onwards and counts unique
/// checkpoint epoch commitments found in the logs.
fn count_checkpoints_in_asm_logs(
    db: &impl DatabaseBackend,
    height_from: u64,
) -> Result<u64, DisplayedError> {
    let asm_db = db.asm_db();
    let l1_db = db.l1_db();

    // Get the latest ASM state to determine the range to scan
    let latest_asm_state = asm_db
        .get_latest_asm_state()
        .internal_error("Failed to get latest ASM state")?;

    let Some((latest_l1_commitment, _)) = latest_asm_state else {
        // No ASM states in the database
        return Ok(0);
    };

    // Get the canonical block ID at height_from to create a proper commitment.
    let block_id = l1_db
        .get_canonical_blockid_at_height(height_from)
        .internal_error("Failed to get canonical block ID at height")?
        .ok_or_else(|| {
            DisplayedError::UserError(
                format!("No L1 block found at height {}", height_from),
                Box::new(height_from),
            )
        })?;

    let start_l1_commitment = L1BlockCommitment::from_height_u64(height_from, block_id)
        .ok_or_else(|| {
            DisplayedError::InternalError(
                "Invalid height for L1BlockCommitment".to_string(),
                Box::new(height_from),
            )
        })?;

    let mut checkpoint_count = 0u64;

    // Batch size for iteration to avoid loading everything at once
    const BATCH_SIZE: usize = 1000;
    let mut current_l1_commitment = start_l1_commitment;

    loop {
        let asm_states = asm_db
            .get_asm_states_from(current_l1_commitment, BATCH_SIZE)
            .internal_error("Failed to get ASM states from database")?;

        if asm_states.is_empty() {
            break;
        }

        // Process each ASM state's logs
        for (commitment, asm_state) in &asm_states {
            // Only process blocks at or after height_from
            if commitment.height_u64() < height_from {
                continue;
            }

            // Iterate through logs in this ASM state
            for log_entry in asm_state.logs() {
                // Try to parse as CheckpointUpdate
                if log_entry.try_into_log::<CheckpointUpdate>().is_ok() {
                    checkpoint_count += 1;
                }
            }
        }

        let (next_l1_commitment, _) = asm_states.last().unwrap();
        if *next_l1_commitment >= latest_l1_commitment {
            break;
        }

        current_l1_commitment = *next_l1_commitment;
    }

    Ok(checkpoint_count)
}

/// Get a checkpoint entry at a specific index.
///
/// Returns `None` if no checkpoint exists at that index.
#[expect(deprecated, reason = "legacy old code is retained for compatibility")]
pub(crate) fn get_checkpoint_at_index(
    db: &impl DatabaseBackend,
    index: u64,
) -> Result<Option<CheckpointEntry>, DisplayedError> {
    #[expect(deprecated, reason = "legacy old code is retained for compatibility")]
    let chkpt_db = db.checkpoint_db();
    #[expect(deprecated, reason = "legacy old code is retained for compatibility")]
    chkpt_db
        .get_checkpoint(index)
        .internal_error(format!("Failed to get checkpoint at index {}", index))
}

/// Get latest checkpoint entry.
#[expect(deprecated, reason = "legacy old code is retained for compatibility")]
pub(crate) fn get_latest_checkpoint_entry(
    db: &impl DatabaseBackend,
) -> Result<CheckpointEntry, DisplayedError> {
    #[expect(deprecated, reason = "legacy old code is retained for compatibility")]
    let chkpt_db = db.checkpoint_db();
    #[expect(deprecated, reason = "legacy old code is retained for compatibility")]
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
    #[expect(deprecated, reason = "legacy old code is retained for compatibility")]
    let chkpt_db = db.checkpoint_db();
    #[expect(deprecated, reason = "legacy old code is retained for compatibility")]
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
    #[expect(deprecated, reason = "legacy old code is retained for compatibility")]
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
    #[expect(deprecated, reason = "legacy old code is retained for compatibility")]
    let chkpt_db = db.checkpoint_db();
    #[expect(deprecated, reason = "legacy old code is retained for compatibility")]
    let last_idx = chkpt_db
        .get_last_checkpoint_idx()
        .internal_error("Failed to get last checkpoint index")?
        .expect("valid checkpoint index");

    let expected_checkpoints_count = last_idx + 1;
    let mut checkpoint_commitments = Vec::new();
    for idx in 0..=last_idx {
        #[expect(deprecated, reason = "legacy old code is retained for compatibility")]
        let entry = chkpt_db
            .get_checkpoint(idx)
            .internal_error(format!("Failed to get checkpoint at index {idx}"))?;

        if let Some(checkpoint_entry) = entry {
            #[expect(deprecated, reason = "legacy old code is retained for compatibility")]
            checkpoint_commitments.push(checkpoint_entry.checkpoint.commitment().clone());
        }
    }
    let checkpoints_found_in_db = checkpoint_commitments.len() as u64;

    // Count unique checkpoints found in ASM logs from L1 blocks
    let checkpoints_in_l1_blocks = count_checkpoints_in_asm_logs(db, args.height_from)?;
    let unexpected_checkpoints_info: Vec<UnexpectedCheckpointInfo> = Vec::new();

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
    #[expect(deprecated, reason = "legacy old code is retained for compatibility")]
    let chkpt_db = db.checkpoint_db();
    let epoch_idx = args.epoch_index;

    #[expect(deprecated, reason = "legacy old code is retained for compatibility")]
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

    #[expect(deprecated, reason = "legacy old code is retained for compatibility")]
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
