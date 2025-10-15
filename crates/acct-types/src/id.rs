use std::fmt;

use int_enum::IntEnum;

use crate::impl_opaque_thin_wrapper;

type RawAccountId = [u8; 32];

/// Universal account identifier.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
#[repr(transparent)]
pub struct AccountId(RawAccountId);

impl_opaque_thin_wrapper!(AccountId => RawAccountId);

type RawAccountSerial = u32;

/// Incrementally assigned account serial number.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
#[repr(transparent)]
pub struct AccountSerial(RawAccountSerial);

impl_opaque_thin_wrapper!(AccountSerial => RawAccountSerial);

impl AccountSerial {
    pub fn incr(self) -> AccountSerial {
        if *self.inner() == RawAccountSerial::MAX {
            panic!("acctsys: reached max serial number");
        }

        AccountSerial::new(self.inner() + 1)
    }
}

type RawSubjectId = [u8; 32];

/// Identifier for a "subject" within the scope of an execution environment.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
#[repr(transparent)]
pub struct SubjectId(RawSubjectId);

impl_opaque_thin_wrapper!(SubjectId => RawSubjectId);

/// Raw primitive version of an account ID.  Defined here for convenience.
pub type RawAccountTypeId = u16;

/// Distinguishes between account types.
#[repr(u16)]
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, IntEnum)]
pub enum AccountTypeId {
    /// "Inert" account type for a stub that exists but does nothing, but store
    /// balance.
    Empty = 0,

    /// Snark accounts.
    Snark = 1,
}

impl fmt::Display for AccountTypeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            AccountTypeId::Empty => "empty",
            AccountTypeId::Snark => "snark",
        };
        write!(f, "{}", s)
    }
}
