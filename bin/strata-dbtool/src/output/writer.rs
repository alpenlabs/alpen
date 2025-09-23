use serde::Serialize;
use strata_db::types::L1BundleStatus;
use strata_primitives::{buf::Buf32, l1::payload::L1Payload};

use super::{helpers::porcelain_field, traits::Formattable};

/// Summary information for writer database
#[derive(Serialize)]
pub(crate) struct WriterSummary {
    pub(crate) total_payload_entries: u64,
    pub(crate) total_intent_entries: u64,
    pub(crate) checkpoints_with_l1_entries: u64,
    pub(crate) checkpoints_without_l1_entries: u64,
    pub(crate) total_checkpoints: u64,
}

impl Formattable for WriterSummary {
    fn format_porcelain(&self) -> String {
        [
            porcelain_field("total_payload_entries", self.total_payload_entries),
            porcelain_field("total_intent_entries", self.total_intent_entries),
            porcelain_field("total_checkpoints", self.total_checkpoints),
            porcelain_field(
                "checkpoints_with_l1_entries",
                self.checkpoints_with_l1_entries,
            ),
            porcelain_field(
                "checkpoints_without_l1_entries",
                self.checkpoints_without_l1_entries,
            ),
        ]
        .join("\n")
    }
}

/// Individual writer payload information
#[derive(Serialize)]
pub(crate) struct WriterPayloadInfo {
    pub(crate) index: u64,
    pub(crate) status: L1BundleStatus,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) payloads: Vec<L1Payload>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) commit_txid: Option<Buf32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) reveal_txid: Option<Buf32>,
}

impl Formattable for WriterPayloadInfo {
    fn format_porcelain(&self) -> String {
        let mut output = Vec::new();

        output.push(porcelain_field("payload_index", self.index));
        output.push(porcelain_field(
            "payload.status",
            format!("{:?}", self.status),
        ));
        output.push(porcelain_field(
            "payload.payload_count",
            self.payloads.len(),
        ));

        // Add individual payload details
        for (payload_index, payload) in self.payloads.iter().enumerate() {
            output.push(porcelain_field(
                &format!("payload.payload_{payload_index}.type"),
                format!("{:?}", payload.payload_type()),
            ));
            output.push(porcelain_field(
                &format!("payload.payload_{payload_index}.data_hash"),
                format!("{:?}", payload.hash()),
            ));
        }

        // Add transaction IDs if available
        if let Some(commit_txid) = &self.commit_txid {
            output.push(porcelain_field(
                "payload.commit_txid",
                format!("{commit_txid:?}"),
            ));
        }
        if let Some(reveal_txid) = &self.reveal_txid {
            output.push(porcelain_field(
                "payload.reveal_txid",
                format!("{reveal_txid:?}"),
            ));
        }

        output.join("\n")
    }
}
