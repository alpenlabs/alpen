use std::convert::From;

pub(crate) type RawProcVersion = u32;

/// Opaque version ID used to know if we need to discard old data and reexecute.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct ProcVersion(RawProcVersion);

impl From<u32> for ProcVersion {
    fn from(value: u32) -> Self {
        Self(value)
    }
}

impl From<ProcVersion> for u32 {
    fn from(value: ProcVersion) -> Self {
        value.0
    }
}
