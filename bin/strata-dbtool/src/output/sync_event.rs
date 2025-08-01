//! Sync event formatting implementations

use strata_primitives::prelude::L1BlockCommitment;
use strata_state::sync_event::SyncEvent;

use super::{helpers::porcelain_field, traits::Formattable};

/// Sync event information displayed to the user
#[derive(serde::Serialize)]
pub(crate) struct SyncEventInfo<'a> {
    pub event_index: u64,
    pub event: &'a SyncEvent,
}

/// Sync events summary information displayed to the user
#[derive(serde::Serialize)]
pub(crate) struct SyncEventsSummaryInfo {
    pub last_event_index: Option<u64>,
    pub expected_l1_blocks_count: u64,
    pub all_sync_events_in_db: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub missing_heights: Vec<u64>,
}

impl<'a> Formattable for SyncEventInfo<'a> {
    fn format_porcelain(&self) -> String {
        let mut output = Vec::new();

        output.push(porcelain_field("sync_event.event_index", self.event_index));

        match self.event {
            SyncEvent::L1Block(l1_commitment) => {
                output.push(porcelain_field("sync_event.event", "L1Block"));
                output.extend(format_l1_block_commitment(
                    l1_commitment,
                    "sync_event.event.l1_block_commitment",
                ));
            }
            SyncEvent::L1Revert(l1_commitment) => {
                output.push(porcelain_field("sync_event.event", "L1Revert"));
                output.extend(format_l1_block_commitment(
                    l1_commitment,
                    "sync_event.event.l1_block_commitment",
                ));
            }
        }

        output.join("\n")
    }
}

impl Formattable for SyncEventsSummaryInfo {
    fn format_porcelain(&self) -> String {
        let mut output = Vec::new();

        if let Some(last_idx) = self.last_event_index {
            output.push(porcelain_field(
                "sync_events_summary.last_event_index",
                last_idx,
            ));
        }

        output.push(porcelain_field(
            "sync_events_summary.expected_l1_blocks_count",
            self.expected_l1_blocks_count,
        ));

        output.push(porcelain_field(
            "sync_events_summary.all_sync_events_in_db",
            if self.all_sync_events_in_db {
                "true"
            } else {
                "false"
            },
        ));

        // Add missing heights if any
        for height in &self.missing_heights {
            output.push(format!("Missing SyncEvent::L1Block for height {height}"));
        }

        output.join("\n")
    }
}

/// Format L1 block commitment for porcelain output
fn format_l1_block_commitment(l1_ref: &L1BlockCommitment, prefix: &str) -> Vec<String> {
    let mut output = Vec::new();

    output.push(porcelain_field(
        &format!("{prefix}.height"),
        l1_ref.height(),
    ));
    output.push(porcelain_field(
        &format!("{prefix}.blkid"),
        format!("{:?}", l1_ref.blkid()),
    ));

    output
}
