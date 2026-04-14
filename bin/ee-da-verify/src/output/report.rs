//! Output produced by the verifier.

use serde::Serialize;

use super::Formattable;

/// Verifier run report. Stage commits extend this with stage-specific fields.
#[derive(Debug, Serialize)]
pub(crate) struct Report {}

impl Formattable for Report {
    fn format_porcelain(&self) -> String {
        "no stages executed".to_string()
    }
}
