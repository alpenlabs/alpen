//! Output types for terminal-header backfill.

use serde::Serialize;
use strata_identifiers::Epoch;

use super::{helpers::porcelain_field, traits::Formattable};

/// Summary emitted by `backfill-terminal-headers`.
#[derive(Debug, Default, Eq, PartialEq, Serialize)]
pub(crate) struct BackfillTerminalHeadersReport {
    pub(crate) epochs_scanned: u64,
    pub(crate) headers_written: u64,
    pub(crate) headers_skipped: u64,
    pub(crate) headers_not_backfilled: u64,
    pub(crate) missing_observed_payload_epochs: Vec<Epoch>,
}

impl Formattable for BackfillTerminalHeadersReport {
    fn format_porcelain(&self) -> String {
        let missing_epochs = self
            .missing_observed_payload_epochs
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",");

        [
            porcelain_field("epochs_scanned", self.epochs_scanned),
            porcelain_field("headers_written", self.headers_written),
            porcelain_field("headers_skipped", self.headers_skipped),
            porcelain_field("headers_not_backfilled", self.headers_not_backfilled),
            porcelain_field("missing_observed_payload_epochs", missing_epochs),
        ]
        .join("\n")
    }
}
