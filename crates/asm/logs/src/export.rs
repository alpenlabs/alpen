use borsh::{BorshDeserialize, BorshSerialize};
use moho_types::ExportEntry;
use strata_asm_common::AsmLog;
use strata_codec_derive::Codec;
use strata_codec_utils::CodecBorsh;
use strata_msg_fmt::TypeId;

use crate::constants::NEW_EXPORT_ENTRY_LOG_TYPE;

/// Details for an export state update event.
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize, Codec)]
pub struct NewExportEntry {
    /// Export container ID.
    container_id: u16,

    /// Export entry data.
    entry_data: CodecBorsh<ExportEntry>,
}

impl NewExportEntry {
    /// Create a new NewExportEntry instance.
    pub fn new(container_id: u16, entry_data: ExportEntry) -> Self {
        Self {
            container_id,
            entry_data: CodecBorsh::new(entry_data),
        }
    }

    pub fn container_id(&self) -> u16 {
        self.container_id
    }

    pub fn entry_data(&self) -> &ExportEntry {
        self.entry_data.inner()
    }
}

impl AsmLog for NewExportEntry {
    const TY: TypeId = NEW_EXPORT_ENTRY_LOG_TYPE;
}
