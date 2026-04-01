//! Checkpoint transaction extraction helpers for OL DA payload consumption.

use strata_checkpoint_types_ssz::TerminalHeaderComplement;
use strata_identifiers::CheckpointL1Ref;

use crate::{DaExtractorResult, OLDaPayloadV1};

/// Trait that abstracts DA extraction.
pub trait DAExtractor {
    /// Extract DA given a checkpoint reference [`CheckpointL1Ref`].
    fn extract_da(&self, ckpt_ref: &CheckpointL1Ref) -> DaExtractorResult<ExtractedDA>;
}

#[derive(Debug)]
pub struct ExtractedDA {
    payload: OLDaPayloadV1,
    terminal_header_complement: TerminalHeaderComplement,
}

impl ExtractedDA {
    pub fn new(
        payload: OLDaPayloadV1,
        terminal_header_complement: TerminalHeaderComplement,
    ) -> Self {
        Self {
            payload,
            terminal_header_complement,
        }
    }

    pub fn payload(&self) -> &OLDaPayloadV1 {
        &self.payload
    }

    pub fn terminal_header_complement(&self) -> &TerminalHeaderComplement {
        &self.terminal_header_complement
    }

    pub fn into_parts(self) -> (OLDaPayloadV1, TerminalHeaderComplement) {
        (self.payload, self.terminal_header_complement)
    }
}
