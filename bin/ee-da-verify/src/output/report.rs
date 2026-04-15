//! Output produced by the verifier.

use serde::Serialize;
use strata_identifiers::Buf32;

use super::{
    helpers::{format_exec_block, porcelain_field},
    Formattable,
};
use crate::{l1::L1BlockRevealStats, state::AppliedExecBlockRange};

/// Verifier run report. Stage commits extend this with stage-specific fields.
#[derive(Debug, Serialize)]
pub(crate) struct Report {
    pub(crate) fetched_block_count: u64,
    pub(crate) blocks_with_reveals: Vec<L1BlockRevealStats>,
    pub(crate) envelope_count: u64,
    pub(crate) blobs_reassembled: u64,
    pub(crate) final_state_root: Buf32,
    pub(crate) applied_range: Option<AppliedExecBlockRange>,
}

impl Formattable for Report {
    fn format_porcelain(&self) -> String {
        let blocks_with_reveals_count = self.blocks_with_reveals.len();
        let reveals_found: u64 = self
            .blocks_with_reveals
            .iter()
            .map(|block| block.reveals_found)
            .sum();
        let mut output = vec![
            porcelain_field("fetched_block_count", self.fetched_block_count),
            porcelain_field("blocks_with_reveals_count", blocks_with_reveals_count),
            porcelain_field("reveals_found", reveals_found),
            porcelain_field("envelope_count", self.envelope_count),
            porcelain_field("blobs_reassembled", self.blobs_reassembled),
            porcelain_field("final_state_root", self.final_state_root),
        ];
        if let Some(range) = &self.applied_range {
            output.push(porcelain_field(
                "applied_range.first",
                format_exec_block(range.first),
            ));
            output.push(porcelain_field(
                "applied_range.last",
                format_exec_block(range.last),
            ));
            output.push(porcelain_field("applied_range.count", range.count));
        }
        for (index, block) in self.blocks_with_reveals.iter().enumerate() {
            output.push(porcelain_field(
                &format!("block_{index}.commitment"),
                block.commitment,
            ));
            output.push(porcelain_field(
                &format!("block_{index}.reveals_found"),
                block.reveals_found,
            ));
        }
        output.join("\n")
    }
}
