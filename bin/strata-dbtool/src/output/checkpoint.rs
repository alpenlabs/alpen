//! Checkpoint formatting implementations

use strata_checkpoint_types::EpochSummary;
use strata_identifiers::Epoch;

use super::{helpers::porcelain_field, traits::Formattable};

/// Epoch information displayed to the user
#[derive(serde::Serialize)]
pub(crate) struct EpochInfo<'a> {
    pub epoch: u64,
    pub epoch_summary: &'a EpochSummary,
}

/// Checkpoint information displayed to the user
#[derive(serde::Serialize)]
pub(crate) struct CheckpointInfo {
    pub checkpoint_epoch: u64,
    pub tip_epoch: Epoch,
    pub tip_l1_height: u32,
    pub tip_l2_slot: u64,
    pub tip_l2_blkid: String,
    pub ol_state_diff_len: usize,
    pub ol_logs_len: usize,
    pub proof_len: usize,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intent_index: Option<u64>,
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
    pub checkpoint_epoch: u64,
    pub l1_height: u64,
}

impl<'a> Formattable for EpochInfo<'a> {
    fn format_porcelain(&self) -> String {
        let mut output = Vec::new();

        output.push(porcelain_field("epoch", self.epoch));
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

impl Formattable for CheckpointInfo {
    fn format_porcelain(&self) -> String {
        let mut output = vec![
            porcelain_field("checkpoint_epoch", self.checkpoint_epoch),
            porcelain_field("checkpoint.tip.epoch", self.tip_epoch),
            porcelain_field("checkpoint.tip.l1_height", self.tip_l1_height),
            porcelain_field("checkpoint.tip.l2.slot", self.tip_l2_slot),
            porcelain_field("checkpoint.tip.l2.blkid", &self.tip_l2_blkid),
            porcelain_field(
                "checkpoint.sidecar.ol_state_diff_len",
                self.ol_state_diff_len,
            ),
            porcelain_field("checkpoint.sidecar.ol_logs_len", self.ol_logs_len),
            porcelain_field("checkpoint.proof_len", self.proof_len),
            porcelain_field("checkpoint.status", &self.status),
        ];

        if let Some(intent_index) = self.intent_index {
            output.push(porcelain_field(
                "checkpoint.status.intent_index",
                intent_index,
            ));
        }

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

        for unexpected_checkpoint in &self.unexpected_checkpoints {
            let prefix = format!("unexpected_checkpoint_{}", unexpected_checkpoint.l1_height);
            output.push(porcelain_field(
                &format!("{prefix}.checkpoint_epoch"),
                unexpected_checkpoint.checkpoint_epoch,
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
            porcelain_field("checkpoint_epoch", self.checkpoint_epoch),
            porcelain_field("l1_height", self.l1_height),
        ];
        output.join("\n")
    }
}
