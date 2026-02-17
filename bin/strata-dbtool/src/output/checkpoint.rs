//! Checkpoint formatting implementations

use strata_checkpoint_types::{BatchInfo, BatchTransition, Checkpoint, EpochSummary};
use strata_csm_types::CheckpointL1Ref;
#[expect(deprecated, reason = "legacy old code is retained for compatibility")]
use strata_db_types::types::{CheckpointConfStatus, CheckpointProvingStatus};

use super::{helpers::porcelain_field, traits::Formattable};

/// Epoch information displayed to the user
#[derive(serde::Serialize)]
pub(crate) struct EpochInfo<'a> {
    pub epoch_index: u64,
    pub epoch_summary: &'a EpochSummary,
}

/// Checkpoint information displayed to the user
#[derive(serde::Serialize)]
pub(crate) struct CheckpointInfo<'a> {
    pub checkpoint_index: u64,
    pub checkpoint: &'a Checkpoint,
    #[expect(deprecated, reason = "legacy old code is retained for compatibility")]
    pub confirmation_status: &'a CheckpointConfStatus,
    #[expect(deprecated, reason = "legacy old code is retained for compatibility")]
    pub proving_status: &'a CheckpointProvingStatus,
}

/// Checkpoints summary information displayed to the user
#[derive(serde::Serialize)]
pub(crate) struct CheckpointsSummaryInfo {
    pub expected_checkpoints_count: u64,
    pub checkpoints_found_in_db: u64,
    pub checkpoints_in_l1_blocks: u64,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub unexpected_checkpoints: Vec<UnexpectedCheckpointInfo>,
}

/// Information about unexpected checkpoints found in L1 blocks
#[derive(serde::Serialize)]
pub(crate) struct UnexpectedCheckpointInfo {
    pub checkpoint_index: u64,
    pub l1_height: u64,
}

/// Format batch info for porcelain output
pub(crate) fn format_batch_info(batch_info: &BatchInfo, prefix: &str) -> Vec<String> {
    let mut output = Vec::new();

    output.push(porcelain_field(
        &format!("{prefix}.batch.epoch"),
        batch_info.epoch(),
    ));

    // Combine L1 range start/end into single rows
    output.push(porcelain_field(
        &format!("{prefix}.batch.l1_range"),
        format!(
            "{} - {}",
            batch_info.l1_range.0.height(),
            batch_info.l1_range.1.height()
        ),
    ));
    output.push(porcelain_field(
        &format!("{prefix}.batch.l1_range.start_blkid"),
        format!("{:?}", batch_info.l1_range.0.blkid()),
    ));
    output.push(porcelain_field(
        &format!("{prefix}.batch.l1_range.end_blkid"),
        format!("{:?}", batch_info.l1_range.1.blkid()),
    ));

    // Combine L2 range start/end into single rows
    output.push(porcelain_field(
        &format!("{prefix}.batch.l2_range"),
        format!(
            "{} - {}",
            batch_info.l2_range.0.slot(),
            batch_info.l2_range.1.slot()
        ),
    ));
    output.push(porcelain_field(
        &format!("{prefix}.batch.l2_range.start_blkid"),
        format!("{:?}", batch_info.l2_range.0.blkid()),
    ));
    output.push(porcelain_field(
        &format!("{prefix}.batch.l2_range.end_blkid"),
        format!("{:?}", batch_info.l2_range.1.blkid()),
    ));

    output
}

/// Format batch transition for porcelain output
pub(crate) fn format_batch_transition(
    batch_transition: &BatchTransition,
    prefix: &str,
) -> Vec<String> {
    let mut output = Vec::new();

    output.push(porcelain_field(
        &format!("{prefix}.batch_transition.chainstate.pre_root"),
        format!(
            "{:?}",
            batch_transition.chainstate_transition.pre_state_root
        ),
    ));
    output.push(porcelain_field(
        &format!("{prefix}.batch_transition.chainstate.post_root"),
        format!(
            "{:?}",
            batch_transition.chainstate_transition.post_state_root
        ),
    ));

    output
}

impl<'a> Formattable for EpochInfo<'a> {
    fn format_porcelain(&self) -> String {
        let mut output = Vec::new();

        output.push(porcelain_field(
            "epoch_summary.epoch_index",
            self.epoch_index,
        ));
        output.push(porcelain_field(
            "epoch_summary.epoch",
            self.epoch_summary.epoch(),
        ));

        let epoch_terminal = self.epoch_summary.terminal();
        output.push(porcelain_field(
            "epoch_summary.terminal.slot",
            epoch_terminal.slot(),
        ));
        output.push(porcelain_field(
            "epoch_summary.terminal.blkid",
            format!("{:?}", epoch_terminal.blkid()),
        ));

        let prev_terminal = self.epoch_summary.prev_terminal();
        output.push(porcelain_field(
            "epoch_summary.prev_terminal.slot",
            prev_terminal.slot(),
        ));
        output.push(porcelain_field(
            "epoch_summary.prev_terminal.blkid",
            format!("{:?}", prev_terminal.blkid()),
        ));

        let new_l1_block = self.epoch_summary.new_l1();
        output.push(porcelain_field(
            "epoch_summary.new_l1.height",
            new_l1_block.height(),
        ));
        output.push(porcelain_field(
            "epoch_summary.new_l1.blkid",
            format!("{:?}", new_l1_block.blkid()),
        ));

        output.push(porcelain_field(
            "epoch_summary.final_state",
            format!("{:?}", self.epoch_summary.final_state()),
        ));

        output.join("\n")
    }
}

impl<'a> Formattable for CheckpointInfo<'a> {
    fn format_porcelain(&self) -> String {
        let mut output = Vec::new();

        output.push(porcelain_field("checkpoint_index", self.checkpoint_index));

        let batch_info = self.checkpoint.batch_info();
        output.extend(format_batch_info(batch_info, "checkpoint"));

        let batch_transition = self.checkpoint.batch_transition();
        output.extend(format_batch_transition(batch_transition, "checkpoint"));

        // Format confirmation status
        match self.confirmation_status {
            #[expect(deprecated, reason = "legacy old code is retained for compatibility")]
            CheckpointConfStatus::Pending => {
                output.push(porcelain_field(
                    "checkpoint.confirmation_status",
                    format!("{:?}", self.confirmation_status),
                ));
            }
            #[expect(deprecated, reason = "legacy old code is retained for compatibility")]
            CheckpointConfStatus::Confirmed(ref checkpoint_l1_ref) => {
                output.push(porcelain_field(
                    "checkpoint.confirmation_status",
                    "Confirmed",
                ));
                output.extend(format_checkpoint_l1_ref(
                    checkpoint_l1_ref,
                    "checkpoint.confirmation_status.l1_ref",
                ));
            }
            #[expect(deprecated, reason = "legacy old code is retained for compatibility")]
            CheckpointConfStatus::Finalized(ref checkpoint_l1_ref) => {
                output.push(porcelain_field(
                    "checkpoint.confirmation_status",
                    "Finalized",
                ));
                output.extend(format_checkpoint_l1_ref(
                    checkpoint_l1_ref,
                    "checkpoint.confirmation_status.l1_ref",
                ));
            }
        }

        output.push(porcelain_field(
            "checkpoint.proving_status",
            format!("{:?}", self.proving_status),
        ));

        output.join("\n")
    }
}

impl Formattable for CheckpointsSummaryInfo {
    fn format_porcelain(&self) -> String {
        let mut output = vec![
            porcelain_field(
                "expected_checkpoints_count",
                self.expected_checkpoints_count,
            ),
            porcelain_field("checkpoints_found_in_db", self.checkpoints_found_in_db),
            porcelain_field("checkpoints_in_l1_blocks", self.checkpoints_in_l1_blocks),
        ];

        // Add unexpected checkpoints information if any
        for unexpected_checkpoint in &self.unexpected_checkpoints {
            let prefix = format!("unexpected_checkpoint_{}", unexpected_checkpoint.l1_height);
            output.push(porcelain_field(
                &format!("{prefix}.checkpoint_index"),
                unexpected_checkpoint.checkpoint_index,
            ));
            output.push(porcelain_field(
                &format!("{prefix}.l1_height"),
                unexpected_checkpoint.l1_height,
            ));
        }

        output.join("\n")
    }
}

impl Formattable for UnexpectedCheckpointInfo {
    fn format_porcelain(&self) -> String {
        let output = [
            porcelain_field("checkpoint_index", self.checkpoint_index),
            porcelain_field("l1_height", self.l1_height),
        ];
        output.join("\n")
    }
}

/// Format checkpoint L1 reference for porcelain output
fn format_checkpoint_l1_ref(l1ref: &CheckpointL1Ref, prefix: &str) -> Vec<String> {
    let mut output = Vec::new();

    output.push(porcelain_field(
        &format!("{prefix}.l1_commitment.height"),
        format!("{:?}", l1ref.l1_commitment.height()),
    ));
    output.push(porcelain_field(
        &format!("{prefix}.l1_commitment.blkid"),
        format!("{:?}", l1ref.l1_commitment.blkid()),
    ));
    output.push(porcelain_field(
        &format!("{prefix}.txid"),
        format!("{:?}", l1ref.txid),
    ));
    output.push(porcelain_field(
        &format!("{prefix}.wtxid"),
        format!("{:?}", l1ref.wtxid),
    ));

    output
}
