//! L2 block formatting implementations

use strata_db::traits::BlockStatus;
use strata_ol_chain_types::{L2Header, SignedL2BlockHeader};
use strata_primitives::{l1::L1BlockId, l2::L2BlockId};

use super::{
    helpers::{porcelain_field, porcelain_optional},
    traits::Formattable,
};

/// L2 Block information displayed to the user
#[derive(serde::Serialize)]
pub(crate) struct L2BlockInfo<'a> {
    pub id: &'a L2BlockId,
    pub status: &'a BlockStatus,
    pub header: &'a SignedL2BlockHeader,
    pub l1_segment: Vec<(u64, &'a L1BlockId)>,
}

/// L2 Summary information displayed to the user
#[derive(serde::Serialize)]
pub(crate) struct L2SummaryInfo<'a> {
    pub tip_slot: u64,
    pub tip_block_id: &'a L2BlockId,
    pub earliest_slot: u64,
    pub earliest_block_id: &'a L2BlockId,
    pub last_epoch: Option<u64>,
    pub expected_block_count: u64,
    pub all_blocks_present: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub missing_slots: Vec<u64>,
}

impl<'a> Formattable for L2BlockInfo<'a> {
    fn format_porcelain(&self) -> String {
        let mut output = Vec::new();

        // Basic block info
        output.push(porcelain_field("l2_block.blkid", format!("{:?}", self.id)));
        output.push(porcelain_field(
            "l2_block.status",
            format!("{:?}", self.status),
        ));

        // Header info
        output.push(porcelain_field("l2_block.header.slot", self.header.slot()));
        output.push(porcelain_field(
            "l2_block.header.epoch",
            self.header.epoch(),
        ));
        output.push(porcelain_field(
            "l2_block.header.timestamp",
            self.header.timestamp(),
        ));
        output.push(porcelain_field(
            "l2_block.header.prev_blkid",
            format!("{:?}", self.header.parent()),
        ));
        output.push(porcelain_field(
            "l2_block.header.l1_segment_hash",
            format!("{:?}", self.header.l1_payload_hash()),
        ));
        output.push(porcelain_field(
            "l2_block.header.exec_segment_hash",
            format!("{:?}", self.header.exec_payload_hash()),
        ));
        output.push(porcelain_field(
            "l2_block.header.state_root",
            format!("{:?}", self.header.state_root()),
        ));

        // L1 segment info
        for (height, blkid) in &self.l1_segment {
            output.push(porcelain_field(
                &format!("l2_block.l1_segment.{height}.blkid"),
                format!("{blkid:?}"),
            ));
        }

        output.join("\n")
    }
}

impl<'a> Formattable for L2SummaryInfo<'a> {
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
