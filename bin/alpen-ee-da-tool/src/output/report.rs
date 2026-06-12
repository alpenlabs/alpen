//! Output produced by the verifier.

use std::fmt::{self, Display};

use alpen_ee_da_state_replay::{AppliedExecBlockRange, ReplaySummary};
use serde::Serialize;
use strata_identifiers::Buf32;

use super::{helpers::porcelain_field, Formattable};
use crate::l1::L1ScanStats;

/// Starting state used for replay.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ReplayStart {
    Genesis,
    Snapshot,
}

impl Display for ReplayStart {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Genesis => f.write_str("genesis"),
            Self::Snapshot => f.write_str("snapshot"),
        }
    }
}

/// Verifier run report. Stage commits extend this with stage-specific fields.
#[derive(Debug, Serialize)]
pub(crate) struct Report {
    pub(crate) replay_start: ReplayStart,
    pub(crate) fetched_block_count: u64,
    pub(crate) envelope_count: u64,
    pub(crate) blobs_reassembled: u64,
    pub(crate) reconstructed_state_root: Buf32,
    pub(crate) applied_range: Option<AppliedExecBlockRange>,
    pub(crate) expected_state_root: Option<Buf32>,
    pub(crate) state_root_matches_expected: Option<bool>,
}

/// Inputs used to assemble a [`Report`].
pub(crate) struct ReportInput {
    pub(crate) scan_stats: L1ScanStats,
    pub(crate) envelope_count: u64,
    pub(crate) blobs_reassembled: u64,
    pub(crate) replay_summary: ReplaySummary,
    pub(crate) replay_start: ReplayStart,
    pub(crate) expected_state_root: Option<Buf32>,
}

impl Report {
    /// Assembles a [`Report`] from the pipeline stage outputs.
    pub(crate) fn new(input: ReportInput) -> Self {
        let ReportInput {
            scan_stats,
            envelope_count,
            blobs_reassembled,
            replay_summary,
            replay_start,
            expected_state_root,
        } = input;
        let reconstructed_state_root = replay_summary.final_state_root();
        let state_root_matches_expected =
            expected_state_root.map(|expected| expected == reconstructed_state_root);
        Self {
            replay_start,
            fetched_block_count: scan_stats.fetched_block_count,
            envelope_count,
            blobs_reassembled,
            reconstructed_state_root,
            applied_range: replay_summary.applied().cloned(),
            expected_state_root,
            state_root_matches_expected,
        }
    }
}

impl Formattable for Report {
    fn format_porcelain(&self) -> String {
        let mut output = vec![
            porcelain_field("replay_start", self.replay_start),
            porcelain_field("fetched_block_count", self.fetched_block_count),
            porcelain_field("envelope_count", self.envelope_count),
            porcelain_field("blobs_reassembled", self.blobs_reassembled),
            porcelain_field("reconstructed_state_root", self.reconstructed_state_root),
        ];

        if let Some(range) = &self.applied_range {
            output.push(porcelain_field(
                "applied_range.first_block_num",
                range.first_block_num(),
            ));
            output.push(porcelain_field(
                "applied_range.first_update_seq_no",
                range.first_update_seq_no(),
            ));
            output.push(porcelain_field(
                "applied_range.last_block_num",
                range.last_block_num(),
            ));
            output.push(porcelain_field(
                "applied_range.last_update_seq_no",
                range.last_update_seq_no(),
            ));
            output.push(porcelain_field("applied_range.count", range.count()));
        }

        if let Some(expected) = self.expected_state_root {
            output.push(porcelain_field("expected_state_root", expected));
        }
        if let Some(matches) = self.state_root_matches_expected {
            output.push(porcelain_field("state_root_matches_expected", matches));
        }

        output.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use alpen_ee_da_state_replay::replay_blobs_from_genesis;

    use super::*;

    #[test]
    fn report_compares_reconstructed_root_against_expected_root() {
        let replay_summary =
            replay_blobs_from_genesis("dev", &[]).expect("empty genesis replay must succeed");
        let reconstructed_state_root = replay_summary.final_state_root();
        let report = Report::new(ReportInput {
            scan_stats: L1ScanStats {
                fetched_block_count: 1,
            },
            envelope_count: 0,
            blobs_reassembled: 0,
            replay_summary,
            replay_start: ReplayStart::Genesis,
            expected_state_root: Some(Buf32::from([0x11; 32])),
        });

        assert_eq!(report.reconstructed_state_root, reconstructed_state_root);
        assert_eq!(report.expected_state_root, Some(Buf32::from([0x11; 32])));
        assert_eq!(report.state_root_matches_expected, Some(false));
    }
}
