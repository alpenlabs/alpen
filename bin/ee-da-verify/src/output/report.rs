//! Output produced by the verifier.

use serde::Serialize;

use super::{helpers::porcelain_field, Formattable};
use crate::l1::L1BlockRevealStats;

/// Verifier run report. Stage commits extend this with stage-specific fields.
#[derive(Debug, Serialize)]
pub(crate) struct Report {
    pub(crate) fetched_block_count: u64,
    pub(crate) blocks_with_reveals: Vec<L1BlockRevealStats>,
    pub(crate) envelope_count: u64,
    pub(crate) blobs_reassembled: u64,
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
        ];
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
