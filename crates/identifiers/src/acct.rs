use std::fmt;

use int_enum::IntEnum;
use ssz_derive::{Decode, Encode};

const ACCT_ID_LEN: usize = 32;
const SUBJ_ID_LEN: usize = 32;

/// Total number of system reserved accounts, which is the space where we do special casing of
/// things.
pub const SYSTEM_RESERVED_ACCTS: u32 = 128;

const SPECIAL_ACCT_ID_BYTE: usize = ACCT_ID_LEN - 1;

type RawAccountId = [u8; ACCT_ID_LEN];

/// Universal account identifier.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Decode, Encode)]
#[repr(transparent)]
#[ssz(struct_behaviour = "transparent")]
pub struct AccountId(RawAccountId);

impl_opaque_thin_wrapper!(AccountId => RawAccountId);

impl AccountId {
    /// The "zero" account ID.
    pub const fn zero() -> Self {
        Self([0; ACCT_ID_LEN])
    }

    /// Gets a special account ID for reserved accounts.
    ///
    /// This is permitted to produce the zero ID.
    pub const fn special(b: u8) -> Self {
        let mut buf = [0; ACCT_ID_LEN];
        buf[SPECIAL_ACCT_ID_BYTE] = b;
        Self(buf)
    }

    /// Checks if this is the zero account ID.
    pub fn is_zero(&self) -> bool {
        self.0.iter().all(|b| *b == 0)
    }

    /// Checks if this is a special account ID.
    ///
    /// This includes the zero ID.
    pub fn is_special(&self) -> bool {
        self.0[..SPECIAL_ACCT_ID_BYTE].iter().all(|b| *b == 0)
    }

    /// Checks if this is a particular special account ID.
    ///
    /// This is permitted to check if this is the zero account ID.
    pub fn is_special_id(&self, b: u8) -> bool {
        self.is_special() && self.0[SPECIAL_ACCT_ID_BYTE] == b
    }
}

impl fmt::Display for AccountId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut buf = [0; SUBJ_ID_LEN * 2];
        hex::encode_to_slice(self.0, &mut buf).expect("ident/acct: encode hex");
        // SAFETY: correct lengths
        f.write_str(unsafe { str::from_utf8_unchecked(&buf) })
    }
}

// Manual TreeHash implementation for transparent wrapper
impl<H: tree_hash::TreeHashDigest> tree_hash::TreeHash<H> for AccountId {
    fn tree_hash_type() -> tree_hash::TreeHashType {
        <RawAccountId as tree_hash::TreeHash<H>>::tree_hash_type()
    }

    fn tree_hash_packed_encoding(&self) -> tree_hash::PackedEncoding {
        <RawAccountId as tree_hash::TreeHash<H>>::tree_hash_packed_encoding(&self.0)
    }

    fn tree_hash_packing_factor() -> usize {
        <RawAccountId as tree_hash::TreeHash<H>>::tree_hash_packing_factor()
    }

    fn tree_hash_root(&self) -> H::Output {
        <RawAccountId as tree_hash::TreeHash<H>>::tree_hash_root(&self.0)
    }
}

// Manual DecodeView implementation for transparent wrapper
impl<'a> ssz::view::DecodeView<'a> for AccountId {
    fn from_ssz_bytes(bytes: &'a [u8]) -> Result<Self, ssz::DecodeError> {
        let array: [u8; ACCT_ID_LEN] =
            bytes
                .try_into()
                .map_err(|_| ssz::DecodeError::InvalidByteLength {
                    len: bytes.len(),
                    expected: ACCT_ID_LEN,
                })?;
        Ok(Self(array))
    }
}

type RawAccountSerial = u32;

/// Incrementally assigned account serial number.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Decode, Encode)]
#[repr(transparent)]
#[ssz(struct_behaviour = "transparent")]
pub struct AccountSerial(RawAccountSerial);

impl_opaque_thin_wrapper!(AccountSerial => RawAccountSerial);

impl AccountSerial {
    /// Returns the zero serial.
    pub const fn zero() -> AccountSerial {
        AccountSerial(0)
    }

    /// Returns the one serial.
    pub const fn one() -> AccountSerial {
        AccountSerial(1)
    }

    /// Creates a serial for one of the reserved accounts.
    ///
    /// # Panics
    ///
    /// If the ID provided is outside the valid range.
    pub const fn reserved(b: u8) -> Self {
        assert!(
            (b as RawAccountSerial) < SYSTEM_RESERVED_ACCTS,
            "acct: out of bounds reserved serial"
        );
        Self(b as RawAccountSerial)
    }

    pub fn incr(self) -> AccountSerial {
        if *self.inner() == RawAccountSerial::MAX {
            panic!("acctsys: reached max serial number");
        }

        AccountSerial::new(self.inner() + 1)
    }

    pub fn is_reserved(&self) -> bool {
        self.0 < SYSTEM_RESERVED_ACCTS
    }
}

impl fmt::Display for AccountSerial {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "serial:{}", &self.0)
    }
}

// Manual TreeHash implementation for transparent wrapper
impl<H: tree_hash::TreeHashDigest> tree_hash::TreeHash<H> for AccountSerial {
    fn tree_hash_type() -> tree_hash::TreeHashType {
        <RawAccountSerial as tree_hash::TreeHash<H>>::tree_hash_type()
    }

    fn tree_hash_packed_encoding(&self) -> tree_hash::PackedEncoding {
        <RawAccountSerial as tree_hash::TreeHash<H>>::tree_hash_packed_encoding(&self.0)
    }

    fn tree_hash_packing_factor() -> usize {
        <RawAccountSerial as tree_hash::TreeHash<H>>::tree_hash_packing_factor()
    }

    fn tree_hash_root(&self) -> H::Output {
        <RawAccountSerial as tree_hash::TreeHash<H>>::tree_hash_root(&self.0)
    }
}

// Manual DecodeView implementation for transparent wrapper
impl<'a> ssz::view::DecodeView<'a> for AccountSerial {
    fn from_ssz_bytes(bytes: &'a [u8]) -> Result<Self, ssz::DecodeError> {
        let inner = <RawAccountSerial as ssz::view::DecodeView<'a>>::from_ssz_bytes(bytes)?;
        Ok(Self(inner))
    }
}

type RawSubjectId = [u8; SUBJ_ID_LEN];

/// Identifier for a "subject" within the scope of an execution environment.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Decode, Encode)]
#[repr(transparent)]
#[ssz(struct_behaviour = "transparent")]
pub struct SubjectId(RawSubjectId);

impl_opaque_thin_wrapper!(SubjectId => RawSubjectId);

impl fmt::Display for SubjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut buf = [0; SUBJ_ID_LEN * 2];
        hex::encode_to_slice(self.0, &mut buf).expect("ident/subj: encode hex");
        // SAFETY: correct lengths
        f.write_str(unsafe { str::from_utf8_unchecked(&buf) })
    }
}

// Manual TreeHash implementation for transparent wrapper
impl<H: tree_hash::TreeHashDigest> tree_hash::TreeHash<H> for SubjectId {
    fn tree_hash_type() -> tree_hash::TreeHashType {
        <RawSubjectId as tree_hash::TreeHash<H>>::tree_hash_type()
    }

    fn tree_hash_packed_encoding(&self) -> tree_hash::PackedEncoding {
        <RawSubjectId as tree_hash::TreeHash<H>>::tree_hash_packed_encoding(&self.0)
    }

    fn tree_hash_packing_factor() -> usize {
        <RawSubjectId as tree_hash::TreeHash<H>>::tree_hash_packing_factor()
    }

    fn tree_hash_root(&self) -> H::Output {
        <RawSubjectId as tree_hash::TreeHash<H>>::tree_hash_root(&self.0)
    }
}

// Manual DecodeView implementation for transparent wrapper
impl<'a> ssz::view::DecodeView<'a> for SubjectId {
    fn from_ssz_bytes(bytes: &'a [u8]) -> Result<Self, ssz::DecodeError> {
        let array: [u8; SUBJ_ID_LEN] =
            bytes
                .try_into()
                .map_err(|_| ssz::DecodeError::InvalidByteLength {
                    len: bytes.len(),
                    expected: SUBJ_ID_LEN,
                })?;
        Ok(Self(array))
    }
}

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

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use ssz::{Decode, Encode};
    use strata_test_utils_ssz::ssz_proptest;

    use super::*;

    mod account_id {
        use super::*;

        ssz_proptest!(
            AccountId,
            any::<[u8; ACCT_ID_LEN]>(),
            transparent_wrapper_of(RawAccountId, new)
        );

        #[test]
        fn test_zero_ssz() {
            let zero = AccountId::new([0u8; ACCT_ID_LEN]);
            let encoded = zero.as_ssz_bytes();
            let decoded = AccountId::from_ssz_bytes(&encoded).unwrap();
            assert_eq!(zero, decoded);
        }
    }

    mod account_serial {
        use super::*;

        ssz_proptest!(
            AccountSerial,
            any::<u32>(),
            transparent_wrapper_of(RawAccountSerial, new)
        );

        #[test]
        fn test_zero_ssz() {
            let zero = AccountSerial::new(0);
            let encoded = zero.as_ssz_bytes();
            let decoded = AccountSerial::from_ssz_bytes(&encoded).unwrap();
            assert_eq!(zero, decoded);
        }
    }

    mod subject_id {
        use super::*;

        ssz_proptest!(
            SubjectId,
            any::<[u8; SUBJ_ID_LEN]>(),
            transparent_wrapper_of(RawSubjectId, new)
        );

        #[test]
        fn test_zero_ssz() {
            let zero = SubjectId::new([0u8; SUBJ_ID_LEN]);
            let encoded = zero.as_ssz_bytes();
            let decoded = SubjectId::from_ssz_bytes(&encoded).unwrap();
            assert_eq!(zero, decoded);
        }
    }
}
