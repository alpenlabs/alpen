use serde::Serialize;
use strata_db::types::L1TxStatus;
use strata_primitives::buf::Buf32;

use super::{helpers::porcelain_field, traits::Formattable};

/// Summary information for broadcaster database
#[derive(Serialize)]
pub(crate) struct BroadcasterSummary {
    pub(crate) total_tx_entries: u64,
    pub(crate) unpublished_count: u64,
    pub(crate) published_count: u64,
    pub(crate) confirmed_count: u64,
    pub(crate) finalized_count: u64,
    pub(crate) invalid_inputs_count: u64,
}

/// Individual broadcaster transaction information
#[derive(Serialize)]
pub(crate) struct BroadcasterTxInfo<'a> {
    pub(crate) index: u64,
    pub(crate) txid: Buf32,
    pub(crate) status: &'a L1TxStatus,
    #[serde(skip_serializing_if = "<[_]>::is_empty")]
    pub(crate) raw_tx: &'a [u8],
}

impl Formattable for BroadcasterSummary {
    fn format_porcelain(&self) -> String {
        [
            porcelain_field("total_tx_entries", self.total_tx_entries),
            porcelain_field("unpublished_count", self.unpublished_count),
            porcelain_field("published_count", self.published_count),
            porcelain_field("confirmed_count", self.confirmed_count),
            porcelain_field("finalized_count", self.finalized_count),
            porcelain_field("invalid_inputs_count", self.invalid_inputs_count),
        ]
        .join("\n")
    }
}

impl<'a> Formattable for BroadcasterTxInfo<'a> {
    fn format_porcelain(&self) -> String {
        [
            porcelain_field("tx_index", self.index),
            porcelain_field("txid", format!("{:?}", self.txid)),
            porcelain_field("tx.status", format!("{:?}", self.status)),
            porcelain_field("tx.raw_tx.len", format!("{:?} bytes", self.raw_tx.len())),
        ]
        .join("\n")
    }
}
