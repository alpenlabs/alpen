use std::{fmt, mem};

use int_enum::IntEnum;

use crate::{errors::AcctError, impl_thin_wrapper};

type RawAcctId = [u8; 32];

/// Universal account identifier.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
#[repr(transparent)]
pub struct AcctId(RawAcctId);

impl_thin_wrapper!(AcctId => RawAcctId);

type RawAcctSerial = u32;

/// Incrementally assigned account serial number.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
#[repr(transparent)]
pub struct AcctSerial(RawAcctSerial);

impl_thin_wrapper!(AcctSerial => RawAcctSerial);

impl AcctSerial {
    pub fn incr(self) -> AcctSerial {
        if *self.inner() == RawAcctSerial::MAX {
            panic!("acctsys: reached max serial number");
        }

        AcctSerial::new(self.inner() + 1)
    }
}

type RawSubjectId = [u8; 32];

/// Identifier for a "subject" within the scope of an execution environment.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
#[repr(transparent)]
pub struct SubjectId(RawSubjectId);

impl_thin_wrapper!(SubjectId => RawSubjectId);

/// Raw primitive version of an account ID.  Defined here for convenience.
pub type RawAcctTypeId = u16;

/// Distinguishes between account types.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, IntEnum)]
#[repr(u16)]
pub enum AcctTypeId {
    /// "Inert" account type for a stub that exists but does nothing, but store
    /// balance.
    Empty = 0,

    /// Snark accounts.
    Snark = 1,
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
