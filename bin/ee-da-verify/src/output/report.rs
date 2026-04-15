//! Output produced by the verifier.

use ee_da_replay::{AppliedExecBlockRange, ReplaySummary};
use serde::Serialize;
use strata_identifiers::Buf32;

use super::{helpers::porcelain_field, Formattable};
use crate::l1::{L1BlockEnvelopeStats, L1ScanStats};

/// Verifier run report. Stage commits extend this with stage-specific fields.
#[derive(Debug, Serialize)]
pub(crate) struct Report {
    pub(crate) fetched_block_count: u64,
    pub(crate) blocks_with_envelopes: Vec<L1BlockEnvelopeStats>,
    pub(crate) envelope_count: u64,
    pub(crate) blobs_reassembled: u64,
    pub(crate) final_state_root: Buf32,
    pub(crate) applied_range: Option<AppliedExecBlockRange>,
    pub(crate) expected_state_root: Option<Buf32>,
    pub(crate) state_root_matches_expected: Option<bool>,
}

impl Report {
    /// Assembles a [`Report`] from the pipeline stage outputs.
    pub(crate) fn new(
        scan_stats: L1ScanStats,
        envelope_count: u64,
        blobs_reassembled: u64,
        replay_summary: ReplaySummary,
        expected_state_root: Option<Buf32>,
    ) -> Self {
        let final_state_root = replay_summary.final_state_root();
        let state_root_matches_expected =
            expected_state_root.map(|expected| expected == final_state_root);
        Self {
            fetched_block_count: scan_stats.fetched_block_count,
            blocks_with_envelopes: scan_stats.blocks_with_envelopes,
            envelope_count,
            blobs_reassembled,
            final_state_root,
            applied_range: replay_summary.applied().cloned(),
            expected_state_root,
            state_root_matches_expected,
        }
    }
}

impl Formattable for Report {
    fn format_porcelain(&self) -> String {
        let blocks_with_envelopes_count = self.blocks_with_envelopes.len();
        let envelopes_found: u64 = self
            .blocks_with_envelopes
            .iter()
            .map(|block| block.envelopes_found)
            .sum();
        let mut output = vec![
            porcelain_field("fetched_block_count", self.fetched_block_count),
            porcelain_field("blocks_with_envelopes_count", blocks_with_envelopes_count),
            porcelain_field("envelopes_found", envelopes_found),
            porcelain_field("envelope_count", self.envelope_count),
            porcelain_field("blobs_reassembled", self.blobs_reassembled),
            porcelain_field("final_state_root", self.final_state_root),
        ];

        if let Some(range) = &self.applied_range {
            output.push(porcelain_field(
                "applied_range.first_block_num",
                range.first_block_num(),
            ));
            output.push(porcelain_field(
                "applied_range.first_block_hash",
                range.first_block_hash(),
            ));
            output.push(porcelain_field(
                "applied_range.last_block_num",
                range.last_block_num(),
            ));
            output.push(porcelain_field(
                "applied_range.last_block_hash",
                range.last_block_hash(),
            ));
            output.push(porcelain_field("applied_range.count", range.count()));
        }

        if let Some(expected) = self.expected_state_root {
            output.push(porcelain_field("expected_state_root", expected));
        }
        if let Some(matches) = self.state_root_matches_expected {
            output.push(porcelain_field("state_root_matches_expected", matches));
        }

        for (index, block) in self.blocks_with_envelopes.iter().enumerate() {
            output.push(porcelain_field(
                &format!("block_{index}.commitment"),
                block.commitment,
            ));
            output.push(porcelain_field(
                &format!("block_{index}.envelopes_found"),
                block.envelopes_found,
            ));
        }

        output.join("\n")
    }
}
