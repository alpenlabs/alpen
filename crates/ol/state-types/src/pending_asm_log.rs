use strata_asm_manifest_types::AsmLogEntry;
use strata_identifiers::L1Height;

/// A pending ASM log entry buffered in the intraepoch state.
///
/// Mirrors `PendingAsmLogEntry` from the concrete `state-types-v1`, but lives
/// here to keep the accessor trait surface free of dependencies on the concrete
/// state crate. `state-types-v1` provides conversions to/from the SSZ container
/// form.
#[derive(Clone, Debug)]
pub struct PendingAsmLog {
    height: L1Height,
    log: AsmLogEntry,
}

impl PendingAsmLog {
    pub fn new(height: L1Height, log: AsmLogEntry) -> Self {
        Self { height, log }
    }

    pub fn height(&self) -> L1Height {
        self.height
    }

    pub fn log(&self) -> &AsmLogEntry {
        &self.log
    }

    pub fn into_parts(self) -> (L1Height, AsmLogEntry) {
        (self.height, self.log)
    }
}
