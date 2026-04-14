//! Output produced by the verifier.

use serde::Serialize;

use super::Formattable;

/// Verifier run report. Stage commits extend this with stage-specific fields.
#[derive(Debug, Serialize)]
pub(crate) struct Report {
    pub(crate) blocks_fetched: u64,
}

impl Formattable for Report {
    fn format_porcelain(&self) -> String {
        format!("fetched blocks: {}", self.blocks_fetched)
    }
}
