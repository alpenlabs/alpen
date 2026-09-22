//! Tools for the intraepoch state.

use ssz_types::VariableList;
use strata_asm_manifest_types::AsmLogEntry;
use strata_identifiers::L1Height;
use strata_ol_state_types::{PendingAsmLog, StateError};

use crate::ssz_generated::ssz::state::*;

impl IntraepochStateV1 {
    /// Creates a new empty instance.
    pub fn new() -> Self {
        Self::default()
    }

    pub fn pending_asm_logs(&self) -> &[PendingAsmLogEntryV1] {
        &self.pending_asm_logs
    }

    pub fn epoch_log_count(&self) -> u32 {
        self.epoch_log_count
    }

    pub fn epoch_log_payload_bytes(&self) -> u32 {
        self.epoch_log_payload_bytes
    }

    pub fn set_epoch_log_usage(&mut self, count: u32, payload_bytes: u32) {
        self.epoch_log_count = count;
        self.epoch_log_payload_bytes = payload_bytes;
    }

    /// Attempts to append a new pending log entry to the buffer.
    ///
    /// # Errors
    ///
    /// If the buffer is already full.
    pub fn try_append_pending_log(&mut self, ent: PendingAsmLogEntryV1) -> Result<(), StateError> {
        self.pending_asm_logs
            .push(ent)
            .map_err(|_| StateError::PendingAsmLogsFull)
    }

    /// Checks if we've maxed out the number of pending logs.
    pub fn is_pending_logs_full(&self) -> bool {
        self.pending_asm_logs.len() as u64 == MAX_PENDING_ASM_LOGS
    }

    /// Clears the intraepoch state. Called at the epoch boundary.
    pub fn reset(&mut self) {
        self.set_epoch_log_usage(0, 0);
        self.pending_asm_logs = VariableList::empty();
    }
}

impl From<&PendingAsmLogEntryV1> for PendingAsmLog {
    fn from(ent: &PendingAsmLogEntryV1) -> Self {
        PendingAsmLog::new(ent.height, ent.log.clone())
    }
}

impl From<PendingAsmLog> for PendingAsmLogEntryV1 {
    fn from(ent: PendingAsmLog) -> Self {
        let (height, log) = ent.into_parts();
        PendingAsmLogEntryV1::new(height, log)
    }
}

impl Default for IntraepochStateV1 {
    fn default() -> Self {
        Self {
            epoch_log_count: 0,
            epoch_log_payload_bytes: 0,
            pending_asm_logs: VariableList::empty(),
        }
    }
}

impl PendingAsmLogEntryV1 {
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
