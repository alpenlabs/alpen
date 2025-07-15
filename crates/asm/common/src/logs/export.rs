use borsh::{BorshDeserialize, BorshSerialize};
use moho_types::ExportEntry;
use strata_msg_fmt::TypeId;

use crate::logs::{AsmLog, constants::NEW_EXPORT_ENTRY_LOG_TYPE};

/// Details for an export state update event.
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct NewExportEntry {
    /// Export container ID.
    pub container_id: u16,
    /// Export entry data.
    pub entry_data: ExportEntry,
}

impl AsmLog for NewExportEntry {
    const TY: TypeId = NEW_EXPORT_ENTRY_LOG_TYPE;
}
