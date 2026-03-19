use std::collections::{BTreeMap, BTreeSet};

use argh::FromArgs;
use strata_asm_logs::CheckpointTipUpdate;
use strata_checkpoint_types_ssz::CheckpointPayload;
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_db_types::{
    traits::{DatabaseBackend, L1Database, OLCheckpointDatabase},
    types::{L1PayloadIntentIndex, OLCheckpointL1ObservationEntry},
};
use strata_identifiers::{Epoch, EpochCommitment, L1Height, Slot};

use crate::{
    cli::OutputFormat,
    cmd::l1::get_l1_chain_tip,
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

    /// L1 reorg-safe depth used to determine whether an observed checkpoint is confirmed or
    /// finalized
    #[argh(option)]
    pub(crate) l1_reorg_safe_depth: u32,
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

pub(crate) struct CheckpointRecord {
    pub(crate) payload: CheckpointPayload,
    pub(crate) signing: Option<L1PayloadIntentIndex>,
    pub(crate) l1_observation: Option<OLCheckpointL1ObservationEntry>,
}

impl CheckpointRecord {
    pub(crate) fn to_status_record(&self) -> CheckpointStatusRecord {
        CheckpointStatusRecord {
            signing: self.signing,
            l1_observation: self.l1_observation,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CheckpointStatusRecord {
    pub(crate) signing: Option<L1PayloadIntentIndex>,
    pub(crate) l1_observation: Option<OLCheckpointL1ObservationEntry>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CheckpointStatus {
    Unsigned,
    Signed,
    Confirmed,
    Finalized,
}

impl CheckpointStatus {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Unsigned => "Unsigned",
            Self::Signed => "Signed",
            Self::Confirmed => "Confirmed",
            Self::Finalized => "Finalized",
        }
    }
}

pub(crate) fn derive_checkpoint_status(
    status_record: CheckpointStatusRecord,
    current_l1_tip: L1Height,
    l1_reorg_safe_depth: u32,
) -> CheckpointStatus {
    match (status_record.signing, status_record.l1_observation) {
        (None, None) => CheckpointStatus::Unsigned,
        (Some(_), None) => CheckpointStatus::Signed,
        (_, Some(observation)) => {
            let confirmations = current_l1_tip
                .saturating_sub(observation.l1_block.height())
                .saturating_add(1);
            let is_finalized = confirmations >= l1_reorg_safe_depth.max(1);
            if is_finalized {
                CheckpointStatus::Finalized
            } else {
                CheckpointStatus::Confirmed
            }
        }
    }
}

/// Resolves canonical OL checkpoint commitment at an epoch.
pub(crate) fn get_canonical_epoch_commitment_at(
    db: &impl DatabaseBackend,
    epoch: Epoch,
) -> Result<Option<EpochCommitment>, DisplayedError> {
    if epoch == 0 {
        return Ok(None);
    }

    let commitments = db
        .ol_checkpoint_db()
        .get_epoch_commitments_at(epoch)
        .internal_error(format!(
            "Failed to get OL checkpoint commitments at epoch {epoch}"
        ))?;

    Ok(commitments.first().copied())
}

/// Gets the derived checkpoint status for a canonical commitment.
pub(crate) fn get_checkpoint_status_by_commitment(
    db: &impl DatabaseBackend,
    checkpoint_epoch: Epoch,
    commitment: EpochCommitment,
    l1_reorg_safe_depth: u32,
) -> Result<Option<CheckpointStatus>, DisplayedError> {
    let payload = db
        .ol_checkpoint_db()
        .get_checkpoint_payload_entry(commitment)
        .internal_error(format!(
            "Failed to get OL checkpoint payload at epoch {checkpoint_epoch}"
        ))?;
    let signing = db
        .ol_checkpoint_db()
        .get_checkpoint_signing_entry(commitment)
        .internal_error(format!(
            "Failed to get OL checkpoint signing entry at epoch {checkpoint_epoch}"
        ))?;
    let l1_observation = db
        .ol_checkpoint_db()
        .get_checkpoint_l1_observation_entry(commitment)
        .internal_error(format!(
            "Failed to get OL checkpoint L1 observation entry at epoch {checkpoint_epoch}"
        ))?;
    let Some(_) = payload else {
        return Ok(None);
    };

    let current_l1_tip = get_l1_chain_tip(db)?.0;
    let status_record = CheckpointStatusRecord {
        signing,
        l1_observation,
    };
    Ok(Some(derive_checkpoint_status(
        status_record,
        current_l1_tip,
        l1_reorg_safe_depth,
    )))
}

/// Gets the derived checkpoint status for an epoch from OL checkpoint DB facts.
pub(crate) fn get_checkpoint_status_at_epoch(
    db: &impl DatabaseBackend,
    checkpoint_epoch: Epoch,
    l1_reorg_safe_depth: u32,
) -> Result<Option<CheckpointStatus>, DisplayedError> {
    let Some(commitment) = get_canonical_epoch_commitment_at(db, checkpoint_epoch)? else {
        return Ok(None);
    };
    get_checkpoint_status_by_commitment(db, checkpoint_epoch, commitment, l1_reorg_safe_depth)
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
) -> Result<Option<CheckpointRecord>, DisplayedError> {
    let Some(commitment) = get_canonical_epoch_commitment_at(db, epoch)? else {
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

    let signing = db
        .ol_checkpoint_db()
        .get_checkpoint_signing_entry(commitment)
        .internal_error(format!(
            "Failed to get OL checkpoint signing entry at epoch {epoch}"
        ))?;

    let l1_observation = db
        .ol_checkpoint_db()
        .get_checkpoint_l1_observation_entry(commitment)
        .internal_error(format!(
            "Failed to get OL checkpoint L1 observation entry at epoch {epoch}"
        ))?;

    Ok(Some(CheckpointRecord {
        payload,
        signing,
        l1_observation,
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

/// Gets the latest checkpoint epoch commitment whose derived status is `Finalized`.
pub(crate) fn get_latest_finalized_checkpoint_epoch(
    db: &impl DatabaseBackend,
    l1_reorg_safe_depth: u32,
) -> Result<Option<EpochCommitment>, DisplayedError> {
    let Some((start_epoch, end_epoch)) = get_checkpoint_epoch_range(db)? else {
        return Ok(None);
    };

    for epoch in (start_epoch..=end_epoch).rev() {
        let Some(status) = get_checkpoint_status_at_epoch(db, epoch, l1_reorg_safe_depth)? else {
            continue;
        };
        if status == CheckpointStatus::Finalized {
            return get_canonical_epoch_commitment_at(db, epoch);
        }
    }

    Ok(None)
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

    let current_l1_tip = get_l1_chain_tip(db)?.0;
    let tip = checkpoint.payload.new_tip();
    let derived_status = derive_checkpoint_status(
        checkpoint.to_status_record(),
        current_l1_tip,
        args.l1_reorg_safe_depth,
    );
    let intent_index = if derived_status == CheckpointStatus::Signed {
        checkpoint.signing
    } else {
        None
    };

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
        status: derived_status.as_str().to_string(),
        intent_index,
        observed_l1_height: checkpoint
            .l1_observation
            .map(|observation| observation.l1_block.height()),
        observed_l1_blkid: checkpoint
            .l1_observation
            .map(|observation| *observation.l1_block.blkid()),
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

    let epoch_range = get_checkpoint_epoch_range(db)?;
    let expected_checkpoints_count = epoch_range.map(|(_, end)| u64::from(end)).unwrap_or(0);

    // Collect all epochs that have a checkpoint in the OL checkpoint DB.
    let mut epochs_in_db = BTreeSet::new();
    let mut checkpoints_found_in_db = 0u64;
    if let Some((start_epoch, end_epoch)) = epoch_range {
        for epoch in start_epoch..=end_epoch {
            let Some(checkpoint) = get_checkpoint_at_epoch(db, epoch)? else {
                continue;
            };
            epochs_in_db.insert(epoch);
            if checkpoint.payload.new_tip().l1_height() >= start_height {
                checkpoints_found_in_db += 1;
            }
        }
    }

    // Track L1-observed checkpoints that do not currently exist in OL checkpoint DB.
    let unexpected_checkpoints_info: Vec<UnexpectedCheckpointInfo> = checkpoint_epochs_in_l1
        .iter()
        .filter(|(epoch, _)| !epochs_in_db.contains(epoch))
        .map(|(&epoch, &l1_height)| UnexpectedCheckpointInfo {
            checkpoint_epoch: epoch,
            l1_height,
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
    let epoch = args.epoch;
    let commitment = get_canonical_epoch_commitment_at(db, epoch)?.ok_or_else(|| {
        DisplayedError::UserError(
            "No epoch summary found for epoch".to_string(),
            Box::new(epoch),
        )
    })?;

    let epoch_summary = db
        .ol_checkpoint_db()
        .get_epoch_summary(commitment)
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

#[cfg(test)]
mod tests {
    use strata_identifiers::{L1BlockCommitment, L1BlockId};

    use super::*;

    fn observation(height: L1Height) -> OLCheckpointL1ObservationEntry {
        OLCheckpointL1ObservationEntry::new(L1BlockCommitment::new(height, L1BlockId::default()))
    }

    #[test]
    fn derive_status_unsigned_when_no_signing_or_observation() {
        let status_record = CheckpointStatusRecord {
            signing: None,
            l1_observation: None,
        };
        let status = derive_checkpoint_status(status_record, 100, 6);
        assert_eq!(status, CheckpointStatus::Unsigned);
    }

    #[test]
    fn derive_status_signed_when_signing_exists_without_observation() {
        let status_record = CheckpointStatusRecord {
            signing: Some(7),
            l1_observation: None,
        };
        let status = derive_checkpoint_status(status_record, 100, 6);
        assert_eq!(status, CheckpointStatus::Signed);
    }

    #[test]
    fn derive_status_confirmed_when_observed_but_below_depth() {
        let status_record = CheckpointStatusRecord {
            signing: Some(7),
            l1_observation: Some(observation(100)),
        };
        let status = derive_checkpoint_status(status_record, 103, 6);
        assert_eq!(status, CheckpointStatus::Confirmed);
    }

    #[test]
    fn derive_status_finalized_when_observed_at_or_above_depth() {
        let status_record = CheckpointStatusRecord {
            signing: Some(7),
            l1_observation: Some(observation(100)),
        };
        let status = derive_checkpoint_status(status_record, 105, 6);
        assert_eq!(status, CheckpointStatus::Finalized);
    }

    #[test]
    fn derive_status_confirmed_when_observed_without_signing() {
        let status_record = CheckpointStatusRecord {
            signing: None,
            l1_observation: Some(observation(100)),
        };
        let status = derive_checkpoint_status(status_record, 103, 6);
        assert_eq!(status, CheckpointStatus::Confirmed);
    }

    #[test]
    fn derive_status_finalized_when_depth_is_zero() {
        let status_record = CheckpointStatusRecord {
            signing: Some(7),
            l1_observation: Some(observation(100)),
        };
        let status = derive_checkpoint_status(status_record, 100, 0);
        assert_eq!(status, CheckpointStatus::Finalized);
    }
}
