use std::{fmt, mem};

/// Universal account identifier.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct AcctId([u8; 32]);

/// Incrementally assigned account serial number.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct AcctSerial(u32);

/// Identifier for a "subject" within the scope of an execution environment.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct SubjectId([u8; 32]);

/// Raw primitive version of an account ID.  Defined here for convenience.
pub type RawAcctTypeId = u16;

/// Distingushes between account types.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
#[repr(u16)]
pub enum AcctTypeId {
    Empty = 0,
    Snark = 1,
}

impl From<RawAcctTypeId> for AcctTypeId {
    fn from(value: RawAcctTypeId) -> Self {
        // SAFETY: this is safe, right?
        unsafe { mem::transmute(value) }
    }
}

impl fmt::Display for AcctTypeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            AcctTypeId::Empty => "empty",
            AcctTypeId::Snark => "snark",
        };
        write!(f, "{}", s)
    }
}
