use argh::FromArgs;
use strata_asm_logs::CheckpointUpdate;
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_db_types::{
    traits::{AsmDatabase, DatabaseBackend, L1Database, OLCheckpointDatabase},
    types::{OLCheckpointEntry, OLCheckpointStatus},
};
use strata_identifiers::Epoch;
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
/// Shows detailed information about a specific OL checkpoint epoch.
pub(crate) struct GetCheckpointArgs {
    /// checkpoint epoch
    #[argh(positional)]
    pub(crate) checkpoint_epoch: Epoch,

    /// output format: "porcelain" (default) or "json"
    #[argh(option, short = 'o', default = "OutputFormat::Porcelain")]
    pub(crate) output_format: OutputFormat,
}

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "get-checkpoints-summary")]
/// Shows a summary of all OL checkpoints in the database.
pub(crate) struct GetCheckpointsSummaryArgs {
    /// start L1 height to query checkpoints from
    #[argh(positional)]
    pub(crate) height_from: u64,

    /// output format: "porcelain" (default) or "json"
    #[argh(option, short = 'o', default = "OutputFormat::Porcelain")]
    pub(crate) output_format: OutputFormat,
}

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "get-epoch-summary")]
/// Shows detailed information about a specific OL epoch summary.
pub(crate) struct GetEpochSummaryArgs {
    /// epoch
    #[argh(positional)]
    pub(crate) epoch: Epoch,

    /// output format: "porcelain" (default) or "json"
    #[argh(option, short = 'o', default = "OutputFormat::Porcelain")]
    pub(crate) output_format: OutputFormat,
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

        let (next_l1_commitment, _) = asm_states.last().expect("asm_states is non-empty");
        if *next_l1_commitment >= latest_l1_commitment {
            break;
        }

        current_l1_commitment = *next_l1_commitment;
    }

    Ok(checkpoint_count)
}

/// Get a checkpoint entry at a specific epoch.
///
/// Returns `None` if no checkpoint exists at that epoch.
pub(crate) fn get_checkpoint_at_epoch(
    db: &impl DatabaseBackend,
    epoch: Epoch,
) -> Result<Option<OLCheckpointEntry>, DisplayedError> {
    db.ol_checkpoint_db()
        .get_checkpoint(epoch)
        .internal_error(format!("Failed to get OL checkpoint at epoch {epoch}"))
}

/// Get the range of checkpoint epochs (0 to latest).
///
/// Returns `None` if no checkpoints exist, otherwise returns `Some((0, latest_epoch))`.
pub(crate) fn get_checkpoint_epoch_range(
    db: &impl DatabaseBackend,
) -> Result<Option<(Epoch, Epoch)>, DisplayedError> {
    db.ol_checkpoint_db()
        .get_last_checkpoint_epoch()
        .internal_error("Failed to get last OL checkpoint epoch")
        .map(|opt| opt.map(|last| (0, last)))
}

/// Get checkpoint details by epoch.
pub(crate) fn get_checkpoint(
    db: &impl DatabaseBackend,
    args: GetCheckpointArgs,
) -> Result<(), DisplayedError> {
    let checkpoint_epoch = args.checkpoint_epoch;
    let entry = get_checkpoint_at_epoch(db, checkpoint_epoch)?.ok_or_else(|| {
        DisplayedError::UserError(
            "No checkpoint found at epoch".to_string(),
            Box::new(checkpoint_epoch),
        )
    })?;

    let tip = entry.checkpoint.new_tip();
    let (status, intent_index) = match &entry.status {
        OLCheckpointStatus::Unsigned => ("Unsigned".to_string(), None),
        OLCheckpointStatus::Signed(idx) => ("Signed".to_string(), Some(*idx)),
    };

    // Create the output data structure
    let checkpoint_info = CheckpointInfo {
        checkpoint_epoch: u64::from(checkpoint_epoch),
        tip_epoch: tip.epoch,
        tip_l1_height: tip.l1_height(),
        tip_l2_slot: tip.l2_commitment().slot(),
        tip_l2_blkid: format!("{:?}", tip.l2_commitment().blkid()),
        ol_state_diff_len: entry.checkpoint.sidecar().ol_state_diff().len(),
        ol_logs_len: entry.checkpoint.sidecar().ol_logs().len(),
        proof_len: entry.checkpoint.proof().len(),
        status,
        intent_index,
    };

    // Use the output utility
    output(&checkpoint_info, args.output_format)
}

/// Get summary of all checkpoints.
pub(crate) fn get_checkpoints_summary(
    db: &impl DatabaseBackend,
    args: GetCheckpointsSummaryArgs,
) -> Result<(), DisplayedError> {
    let Some(last_epoch) = db
        .ol_checkpoint_db()
        .get_last_checkpoint_epoch()
        .internal_error("Failed to get last OL checkpoint epoch")?
    else {
        let summary_info = CheckpointsSummaryInfo {
            expected_checkpoints_count: 0,
            checkpoints_found_in_db: 0,
            checkpoints_in_l1_blocks: count_checkpoints_in_asm_logs(db, args.height_from)?,
            unexpected_checkpoints: Vec::new(),
        };
        return output(&summary_info, args.output_format);
    };

    // Checkpoint for epoch 0 is not expected: checkpoint building starts from epoch 1.
    let expected_checkpoints_count = u64::from(last_epoch);
    let mut checkpoints_found_in_db = 0u64;

    for idx in 0..=last_epoch {
        if get_checkpoint_at_epoch(db, idx)?.is_some() {
            checkpoints_found_in_db += 1;
        }
    }

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
    let epoch = args.epoch;

    let epoch_commitments = db
        .ol_checkpoint_db()
        .get_epoch_commitments_at(u64::from(epoch))
        .internal_error(format!(
            "Failed to get OL epoch commitments for epoch {epoch}"
        ))?;

    if epoch_commitments.is_empty() {
        return Err(DisplayedError::UserError(
            "No epoch summary found for epoch".to_string(),
            Box::new(epoch),
        ));
    }

    let epoch_summary = db
        .ol_checkpoint_db()
        .get_epoch_summary(epoch_commitments[0])
        .internal_error(format!("Failed to get OL epoch summary for epoch {epoch}"))?
        .ok_or_else(|| {
            DisplayedError::UserError(
                format!("No epoch summary found for epoch {epoch}"),
                Box::new(epoch),
            )
        })?;

    // Create the output data structure
    let epoch_info = EpochInfo {
        epoch: u64::from(epoch),
        epoch_summary: &epoch_summary,
    };

    // Use the output utility
    output(&epoch_info, args.output_format)
}
