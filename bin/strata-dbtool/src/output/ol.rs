//! OL block formatting implementations for `get-ol-*` commands.

use strata_db_types::traits::BlockStatus;
use strata_identifiers::OLBlockId;
use strata_primitives::l1::L1BlockId;

use super::{
    helpers::{porcelain_field, porcelain_optional},
    traits::Formattable,
};

/// OL block information displayed to the user.
#[derive(serde::Serialize)]
pub(crate) struct OLBlockInfo<'a> {
    pub id: &'a OLBlockId,
    pub status: &'a BlockStatus,
    pub header_slot: u64,
    pub header_epoch: u64,
    pub header_timestamp: u64,
    pub header_prev_blkid: String,
    pub header_body_root: String,
    pub header_logs_root: String,
    pub header_state_root: String,
    pub manifests: Vec<(u64, &'a L1BlockId)>,
}

/// OL summary information displayed to the user.
#[derive(serde::Serialize)]
pub(crate) struct OLSummaryInfo<'a> {
    pub tip_slot: u64,
    pub tip_block_id: &'a OLBlockId,
    pub earliest_slot: u64,
    pub earliest_block_id: &'a OLBlockId,
    pub last_epoch: Option<u64>,
    pub expected_block_count: u64,
    pub all_blocks_present: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub missing_slots: Vec<u64>,
}

impl<'a> Formattable for OLBlockInfo<'a> {
    fn format_porcelain(&self) -> String {
        let mut output = Vec::new();

        // Basic block info
        output.push(porcelain_field("ol_block.blkid", format!("{:?}", self.id)));
        output.push(porcelain_field(
            "ol_block.status",
            format!("{:?}", self.status),
        ));

        // Header info
        output.push(porcelain_field("ol_block.header.slot", self.header_slot));
        output.push(porcelain_field("ol_block.header.epoch", self.header_epoch));
        output.push(porcelain_field(
            "ol_block.header.timestamp",
            self.header_timestamp,
        ));
        output.push(porcelain_field(
            "ol_block.header.prev_blkid",
            &self.header_prev_blkid,
        ));
        output.push(porcelain_field(
            "ol_block.header.body_root",
            &self.header_body_root,
        ));
        output.push(porcelain_field(
            "ol_block.header.logs_root",
            &self.header_logs_root,
        ));
        output.push(porcelain_field(
            "ol_block.header.state_root",
            &self.header_state_root,
        ));

        // Manifest info (from terminal block l1_update, if present).
        for (height, blkid) in &self.manifests {
            output.push(porcelain_field(
                &format!("ol_block.manifests.{height}.blkid"),
                format!("{blkid:?}"),
            ));
        }

        output.join("\n")
    }
}

impl<'a> Formattable for OLSummaryInfo<'a> {
    fn format_porcelain(&self) -> String {
        let mut output = Vec::new();

        output.push(porcelain_field("tip_slot", self.tip_slot));
        output.push(porcelain_field(
            "tip_block_id",
            format!("{:?}", self.tip_block_id),
        ));
        output.push(porcelain_field("earliest_slot", self.earliest_slot));
        output.push(porcelain_field(
            "earliest_block_id",
            format!("{:?}", self.earliest_block_id),
        ));
        output.push(porcelain_field(
            "last_epoch",
            porcelain_optional(&self.last_epoch),
        ));
        output.push(porcelain_field(
            "expected_block_count",
            self.expected_block_count,
        ));
        output.push(porcelain_field(
            "all_blocks_present",
            super::helpers::porcelain_bool(self.all_blocks_present),
        ));

        // Add missing slot information if any
        for (index, slot) in self.missing_slots.iter().enumerate() {
            output.push(porcelain_field(&format!("missing_slot_{index}"), slot));
        }

        output.join("\n")
    }
}
