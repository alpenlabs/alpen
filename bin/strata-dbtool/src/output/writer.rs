use serde::Serialize;
use strata_crypto::hash;
use strata_csm_types::L1Payload;
use strata_db_types::types::L1BundleStatus;
use strata_primitives::buf::Buf32;

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
    pub(crate) payload: L1Payload,
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

        // Add payload details
        // Concatenate all payload chunks into a single payload for hashing
        let concatenated_payload: Vec<u8> = self.payload.data().iter().flatten().copied().collect();
        let payload_hash = hash::raw(&concatenated_payload);
        output.push(porcelain_field(
            "payload.subproto_id",
            format!("{:?}", self.payload.tag().subproto_id()),
        ));
        output.push(porcelain_field(
            "payload.tx_type",
            format!("{:?}", self.payload.tag().tx_type()),
        ));
        output.push(porcelain_field(
            "payload.data_hash",
            format!("{:?}", payload_hash),
        ));

        // Add transaction IDs if available
        if let Some(commit_txid) = &self.commit_txid {
            output.push(porcelain_field(
                "payload.commit_txid",
                format!("{:?}", commit_txid),
            ));
        }
        if let Some(reveal_txid) = &self.reveal_txid {
            output.push(porcelain_field(
                "payload.reveal_txid",
                format!("{:?}", reveal_txid),
            ));
        }

        output.join("\n")
    }
}
