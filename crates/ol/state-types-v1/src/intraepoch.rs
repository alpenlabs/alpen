//! Tools for the intraepoch state.

use ssz::DecodeError;
use ssz_types::view::ToOwnedSsz;
use ssz_types::{Optional, VariableList};
use strata_asm_manifest_types::AsmLogEntry;
use strata_identifiers::{L1Height, SszDelegate, impl_ssz_via_delegate};
use strata_ol_state_types::{PendingAsmLog, StateError};

use crate::required_fields::require_present;
use crate::ssz_generated::ssz::state::IntraepochStateV1Ssz;
use crate::{MAX_PENDING_ASM_LOGS, PendingAsmLogEntryV1};

/// Intraepoch OL state with its required log buffer present.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IntraepochStateV1 {
    pending_asm_logs: VariableList<PendingAsmLogEntryV1, { MAX_PENDING_ASM_LOGS as usize }>,
}

impl SszDelegate for IntraepochStateV1 {
    type Delegate = IntraepochStateV1Ssz;

    fn into_delegate(self) -> Self::Delegate {
        IntraepochStateV1Ssz {
            pending_asm_logs: Optional::Some(self.pending_asm_logs),
        }
    }

    fn from_delegate(delegate: Self::Delegate) -> Result<Self, DecodeError> {
        Ok(Self {
            pending_asm_logs: require_present(
                delegate.pending_asm_logs,
                "intraepoch.pending_asm_logs",
            )?,
        })
    }
}

impl_ssz_via_delegate!(IntraepochStateV1);

impl ToOwnedSsz<IntraepochStateV1> for IntraepochStateV1 {
    fn to_owned(&self) -> IntraepochStateV1 {
        self.clone()
    }
}

impl IntraepochStateV1 {
    /// Creates a new empty instance.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the buffered ASM logs.
    pub fn pending_asm_logs(&self) -> &[PendingAsmLogEntryV1] {
        &self.pending_asm_logs
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
