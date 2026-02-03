use strata_asm_common::AsmLog;
use strata_codec::Codec;
use strata_codec_utils::CodecRkyv;
use strata_msg_fmt::TypeId;

use crate::constants::NEW_EXPORT_ENTRY_LOG_TYPE;

/// Details for an export state update event.
#[derive(Debug, Clone, Codec)]
pub struct NewExportEntry {
    /// Export container ID.
    container_id: u8,

    /// Export entry data.
    entry_data: CodecRkyv<[u8; 32]>,
}

impl NewExportEntry {
    /// Create a new NewExportEntry instance.
    pub fn new(container_id: u8, entry_data: [u8; 32]) -> Self {
        Self {
            container_id,
            entry_data: CodecRkyv::new(entry_data),
        }
    }

    pub fn container_id(&self) -> u8 {
        self.container_id
    }

    pub fn entry_data(&self) -> &[u8; 32] {
        self.entry_data.inner()
    }
}

impl AsmLog for NewExportEntry {
    const TY: TypeId = NEW_EXPORT_ENTRY_LOG_TYPE;
}
