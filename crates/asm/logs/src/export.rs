use borsh::{BorshDeserialize, BorshSerialize};
use moho_types::ExportEntry;
use strata_asm_common::AsmLog;
use strata_msg_fmt::TypeId;

use crate::constants::NEW_EXPORT_ENTRY_LOG_TYPE;

/// Details for an export state update event.
///
/// TODO(PaaS-refactor): Temporarily removed BorshSerialize/BorshDeserialize derives
/// because moho-types::ExportEntry doesn't implement these traits. This is a known
/// issue from the main branch rebase. Once moho-types is updated to support Borsh
/// serialization, restore these derives.
///
/// See: https://github.com/alpenlabs/moho (needs Borsh support)
#[derive(Debug, Clone)]
pub struct NewExportEntry {
    /// Export container ID.
    pub container_id: u16,
    /// Export entry data.
    pub entry_data: ExportEntry,
}

impl NewExportEntry {
    /// Create a new NewExportEntry instance.
    pub fn new(container_id: u16, entry_data: ExportEntry) -> Self {
        Self {
            container_id,
            entry_data,
        }
    }
}

// TODO(PaaS-refactor): Stub implementations for Borsh traits
// These are temporary workarounds until moho-types supports Borsh serialization
impl BorshSerialize for NewExportEntry {
    fn serialize<W: std::io::Write>(&self, _writer: &mut W) -> std::io::Result<()> {
        unimplemented!("NewExportEntry Borsh serialization requires moho-types Borsh support")
    }
}

impl BorshDeserialize for NewExportEntry {
    fn deserialize_reader<R: std::io::Read>(_reader: &mut R) -> std::io::Result<Self> {
        unimplemented!("NewExportEntry Borsh deserialization requires moho-types Borsh support")
    }
}

impl AsmLog for NewExportEntry {
    const TY: TypeId = NEW_EXPORT_ENTRY_LOG_TYPE;
}
