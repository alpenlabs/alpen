//! Tools for the intraepoch state.

use ssz_types::VariableList;
use strata_asm_manifest_types::AsmLogEntry;
use strata_identifiers::L1Height;

use crate::ssz_generated::ssz::state::*;

impl IntraepochState {
    /// Creates a new empty instance.
    pub fn new() -> Self {
        Self::default()
    }

    pub fn pending_asm_logs(&self) -> &[PendingAsmLogEntry] {
        &self.pending_asm_logs
    }

    /// Attempts to append a new pending log entry to the buffer, returning
    /// if success.
    pub fn try_append_pending_log(&mut self, ent: PendingAsmLogEntry) -> bool {
        self.pending_asm_logs.push(ent).is_ok()
    }

    /// Checks if we've maxed out the number of pending logs.
    pub fn is_pending_logs_full(&self) -> bool {
        self.pending_asm_logs.len() as u64 == MAX_PENDING_ASM_LOGS
    }
}

impl Default for IntraepochState {
    fn default() -> Self {
        Self {
            pending_asm_logs: VariableList::empty(),
        }
    }
}

impl PendingAsmLogEntry {
    pub fn new(height: L1Height, log: AsmLogEntry) -> Self {
        Self { height, log }
    }

    pub fn height(&self) -> L1Height {
        self.height
    }

    pub fn log(&self) -> &AsmLogEntry {
        &self.log
    }

    pub fn into_log(self) -> AsmLogEntry {
        self.log
    }
}
