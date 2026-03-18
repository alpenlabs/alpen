use std::collections::{BTreeMap, BTreeSet};

use argh::FromArgs;
use strata_asm_logs::CheckpointTipUpdate;
use strata_checkpoint_types_ssz::CheckpointPayload;
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_db_types::traits::{DatabaseBackend, L1Database, OLCheckpointDatabase};
use strata_identifiers::{Epoch, L1Height, Slot};

use crate::{
    cli::OutputFormat,
    cmd::l1::get_l1_chain_tip,
    output::{
        checkpoint::{CheckpointInfo, CheckpointsSummaryInfo, EpochInfo, UnexpectedCheckpointInfo},
        output,
    },
};

pub(crate) struct OLCheckpointAtEpoch {
    pub(crate) payload: CheckpointPayload,
    pub(crate) intent_index: Option<u64>,
}

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
    pub(crate) height_from: L1Height,

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
fn collect_checkpoint_epochs_in_l1_logs(
    db: &impl DatabaseBackend,
    height_from: L1Height,
) -> Result<BTreeMap<Epoch, L1Height>, DisplayedError> {
    let l1_db = db.l1_db();
    let (tip_height, _) = get_l1_chain_tip(db)?;

    let start_height = height_from;
    if start_height > tip_height {
        return Ok(BTreeMap::new());
    }

    let mut unique_checkpoint_epochs = BTreeSet::<Epoch>::new();
    let mut checkpoint_epoch_first_seen_heights = BTreeMap::<Epoch, L1Height>::new();

    for height in start_height..=tip_height {
        let Some(block_id) = l1_db
            .get_canonical_blockid_at_height(height)
            .internal_error(format!(
                "Failed to get canonical block ID at height {height}"
            ))?
        else {
            continue;
        };

        let Some(manifest) = l1_db.get_block_manifest(block_id).internal_error(format!(
            "Failed to get L1 block manifest at height {height}"
        ))?
        else {
            continue;
        };

        for log_entry in manifest.logs() {
            if let Ok(update) = log_entry.try_into_log::<CheckpointTipUpdate>() {
                let epoch = update.tip().epoch;
                if unique_checkpoint_epochs.insert(epoch) {
                    checkpoint_epoch_first_seen_heights.insert(epoch, height);
                }
            }
        }
    }

    Ok(checkpoint_epoch_first_seen_heights)
}

/// Get a checkpoint entry at a specific epoch.
///
/// Returns `None` if no checkpoint exists at that epoch.
pub(crate) fn get_checkpoint_at_epoch(
    db: &impl DatabaseBackend,
    epoch: Epoch,
) -> Result<Option<OLCheckpointAtEpoch>, DisplayedError> {
    // OL checkpoints are created from epoch 1
    if epoch == 0 {
        return Ok(None);
    }

    let commitments = db
        .ol_checkpoint_db()
        .get_epoch_commitments_at(epoch)
        .internal_error(format!(
            "Failed to get OL checkpoint commitments at epoch {epoch}"
        ))?;

    let Some(commitment) = commitments.first().copied() else {
        return Ok(None);
    };

    let Some(payload) = db
        .ol_checkpoint_db()
        .get_checkpoint_payload_entry(commitment)
        .internal_error(format!(
            "Failed to get OL checkpoint payload at epoch {epoch}"
        ))?
    else {
        return Ok(None);
    };

    let intent_index = db
        .ol_checkpoint_db()
        .get_checkpoint_signing_entry(commitment)
        .internal_error(format!(
            "Failed to get OL checkpoint signing entry at epoch {epoch}"
        ))?;

    Ok(Some(OLCheckpointAtEpoch {
        payload,
        intent_index,
    }))
}

/// Get the range of checkpoint epochs (1 to latest).
///
/// Returns `None` if no checkpoints exist, otherwise returns `Some((1, latest_epoch))`.
pub(crate) fn get_checkpoint_epoch_range(
    db: &impl DatabaseBackend,
) -> Result<Option<(Epoch, Epoch)>, DisplayedError> {
    get_last_ol_checkpoint_epoch(db)
        .map(|opt| opt.and_then(|last_epoch| (last_epoch >= 1).then_some((1, last_epoch))))
}

/// Get last written OL checkpoint payload epoch number.
pub(crate) fn get_last_ol_checkpoint_epoch(
    db: &impl DatabaseBackend,
) -> Result<Option<Epoch>, DisplayedError> {
    db.ol_checkpoint_db()
        .get_last_checkpoint_payload_epoch()
        .internal_error("Failed to get last OL checkpoint epoch")
        .map(|commitment| commitment.map(|commitment| commitment.epoch()))
}

/// Gets the last checkpointed OL slot from checkpoint payload tip data.
pub(crate) fn get_latest_checkpoint_last_slot(
    db: &impl DatabaseBackend,
) -> Result<Slot, DisplayedError> {
    let Some(latest_epoch_commitment) = db
        .ol_checkpoint_db()
        .get_last_checkpoint_payload_epoch()
        .internal_error("Failed to get last checkpoint epoch")?
    else {
        return Ok(0);
    };

    let checkpoint_payload = db
        .ol_checkpoint_db()
        .get_checkpoint_payload_entry(latest_epoch_commitment)
        .internal_error("Failed to get OL checkpoint payload")?
        .ok_or_else(|| {
            DisplayedError::InternalError(
                "Last checkpoint epoch exists but checkpoint payload is missing".to_string(),
                Box::new(latest_epoch_commitment),
            )
        })?;

    Ok(checkpoint_payload.new_tip().l2_commitment().slot())
}

/// Get checkpoint details by epoch.
pub(crate) fn get_checkpoint(
    db: &impl DatabaseBackend,
    args: GetCheckpointArgs,
) -> Result<(), DisplayedError> {
    let checkpoint_epoch = args.checkpoint_epoch;
    let checkpoint = get_checkpoint_at_epoch(db, checkpoint_epoch)?.ok_or_else(|| {
        DisplayedError::UserError(
            "No checkpoint found at epoch".to_string(),
            Box::new(checkpoint_epoch),
        )
    })?;

    let tip = checkpoint.payload.new_tip();
    let (signing_status, intent_index) = match checkpoint.intent_index {
        None => ("Unsigned".to_string(), None),
        Some(idx) => ("Signed".to_string(), Some(idx)),
    };
    let confirmation_status = "N/A".to_string();

    // Create the output data structure
    let checkpoint_info = CheckpointInfo {
        checkpoint_epoch,
        tip_epoch: tip.epoch,
        tip_l1_height: tip.l1_height(),
        tip_ol_slot: tip.l2_commitment().slot(),
        tip_ol_blkid: *tip.l2_commitment().blkid(),
        ol_state_diff_len: checkpoint.payload.sidecar().ol_state_diff().len(),
        ol_logs_len: checkpoint.payload.sidecar().ol_logs().len(),
        proof_len: checkpoint.payload.proof().len(),
        signing_status,
        confirmation_status,
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
    let l1_db = db.l1_db();
    let start_height = args.height_from;

    let (l1_tip_height, _) = get_l1_chain_tip(db)?;

    if start_height > l1_tip_height {
        return Err(DisplayedError::UserError(
            format!("Provided height is above canonical L1 tip {l1_tip_height}"),
            Box::new(args.height_from),
        ));
    }
    if l1_db
        .get_canonical_blockid_at_height(start_height)
        .internal_error(format!(
            "Failed to get canonical block ID at height {}",
            args.height_from
        ))?
        .is_none()
    {
        return Err(DisplayedError::UserError(
            "Provided height is not present in canonical L1 chain".to_string(),
            Box::new(args.height_from),
        ));
    }

    let checkpoint_epochs_in_l1 = collect_checkpoint_epochs_in_l1_logs(db, args.height_from)?;
    let checkpoints_in_l1_blocks = checkpoint_epochs_in_l1.len() as u64;

    // Count checkpoint entries in OL checkpoint DB filtered by checkpoint tip L1 height.
    // Iterate over the canonical checkpoint epoch range (starting at epoch 1).
    let mut checkpoints_found_in_db = 0u64;
    if let Some((start_epoch, end_epoch)) = get_checkpoint_epoch_range(db)? {
        for epoch in start_epoch..=end_epoch {
            let Some(checkpoint) = get_checkpoint_at_epoch(db, epoch)? else {
                continue;
            };
            if checkpoint.payload.new_tip().l1_height() >= start_height {
                checkpoints_found_in_db += 1;
            }
        }
    }

    // Keep expected scope aligned with the DB-filtered range.
    let expected_checkpoints_count = checkpoints_found_in_db;

    // Track L1-observed checkpoints that do not currently exist in OL checkpoint DB.
    let mut unexpected_checkpoints_info: Vec<UnexpectedCheckpointInfo> = Vec::new();
    for (epoch, l1_height) in checkpoint_epochs_in_l1 {
        if get_checkpoint_at_epoch(db, epoch)?.is_none() {
            unexpected_checkpoints_info.push(UnexpectedCheckpointInfo {
                checkpoint_epoch: epoch,
                l1_height,
            });
        }
    }

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
        .get_epoch_commitments_at(epoch)
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
        epoch,
        epoch_summary: &epoch_summary,
    };

    // Use the output utility
    output(&epoch_info, args.output_format)
}
