use std::{fmt, mem};

use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use int_enum::IntEnum;
use serde::{Deserialize, Serialize};
use ssz_derive::{Decode, Encode};
use thiserror::Error;

const ACCT_ID_LEN: usize = 32;
pub const SUBJ_ID_LEN: usize = 32;

/// Total number of system reserved accounts, which is the space where we do special casing of
/// things.
pub const SYSTEM_RESERVED_ACCTS: u32 = 128;

const SPECIAL_ACCT_ID_BYTE: usize = ACCT_ID_LEN - 1;

type RawAccountId = [u8; ACCT_ID_LEN];

/// Universal account identifier.
#[derive(
    Copy,
    Clone,
    Debug,
    Eq,
    PartialEq,
    Ord,
    PartialOrd,
    Hash,
    Decode,
    Encode,
    Serialize,
    Deserialize,
    BorshSerialize,
    BorshDeserialize,
)]
pub struct AccountId(#[serde(with = "hex::serde")] RawAccountId);

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

impl_ssz_transparent_byte_array_wrapper!(AccountId, 32);

type RawAccountSerial = u32;

/// Size of RawAccountSerial (u32) in bytes
const RAW_ACCOUNT_SERIAL_LEN: usize = mem::size_of::<RawAccountSerial>();

/// Incrementally assigned account serial number.
#[derive(
    Copy,
    Clone,
    Debug,
    Eq,
    PartialEq,
    Ord,
    PartialOrd,
    Hash,
    Arbitrary,
    Decode,
    Encode,
    BorshSerialize,
    BorshDeserialize,
    Serialize,
    Deserialize,
)]
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

crate::impl_ssz_transparent_wrapper!(AccountSerial, RawAccountSerial, RAW_ACCOUNT_SERIAL_LEN);

type RawSubjectId = [u8; SUBJ_ID_LEN];

/// Identifier for a "subject" within the scope of an execution environment.
#[derive(
    Copy,
    Clone,
    Debug,
    Eq,
    PartialEq,
    Ord,
    PartialOrd,
    Hash,
    Decode,
    Encode,
    Serialize,
    Deserialize,
    Arbitrary,
    BorshSerialize,
    BorshDeserialize,
)]
pub struct SubjectId(#[serde(with = "hex::serde")] RawSubjectId);

impl_opaque_thin_wrapper!(SubjectId => RawSubjectId);

impl fmt::Display for SubjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut buf = [0; SUBJ_ID_LEN * 2];
        hex::encode_to_slice(self.0, &mut buf).expect("ident/subj: encode hex");
        // SAFETY: correct lengths
        f.write_str(unsafe { str::from_utf8_unchecked(&buf) })
    }
}

crate::impl_ssz_transparent_byte_array_wrapper!(SubjectId, 32);

/// Error type for [`SubjectBytes`] operations.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum SubjectIdBytesError {
    /// Subject bytes exceed the maximum allowed length.
    #[error("subject bytes length {0} exceeds maximum length {SUBJ_ID_LEN}")]
    TooLong(usize),
}

/// Variable-length [`SubjectId`] bytes.
///
/// Subject IDs are canonically [`SUBJ_ID_LEN`] bytes per the account system specification, but in
/// practice many subject IDs are shorter. This type stores the variable-length byte representation
/// to optimize DA costs by avoiding unnecessary zero padding in the on-chain deposit descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubjectIdBytes(Vec<u8>);

impl SubjectIdBytes {
    /// Creates a new `SubjectBytes` instance from a byte vector.
    ///
    /// # Errors
    ///
    /// Returns an error if the length exceeds [`SUBJ_ID_LEN`].
    pub fn try_new(bytes: Vec<u8>) -> Result<Self, SubjectIdBytesError> {
        if bytes.len() > SUBJ_ID_LEN {
            return Err(SubjectIdBytesError::TooLong(bytes.len()));
        }
        Ok(Self(bytes))
    }

    /// Returns the raw, unpadded subject bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Converts to a canonical [`SUBJ_ID_LEN`]-byte [`SubjectId`].
    ///
    /// This method allocates a [`SUBJ_ID_LEN`]-byte buffer and zero-pads the stored subject bytes.
    /// The original bytes are copied to the end of the buffer, with any remaining
    /// bytes filled with zeros at the beginning.
    ///
    /// # Example
    ///
    /// If the stored bytes are shorter than [`SUBJ_ID_LEN`], such as `[0xAA, 0xBB, ..., 0xFF]`,
    /// this method returns a [`SUBJ_ID_LEN`]-byte `SubjectId` with leading zeros and the bytes
    /// at the end: `[0x00, 0x00, ..., 0x00, 0xAA, 0xBB, ..., 0xFF]`.
    pub fn to_subject_id(&self) -> SubjectId {
        let mut buf = [0u8; SUBJ_ID_LEN];
        let start = SUBJ_ID_LEN - self.0.len();
        buf[start..].copy_from_slice(&self.0);
        SubjectId::new(buf)
    }

    /// Returns the length of the subject bytes.
    pub const fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns true if the subject bytes are empty.
    pub const fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Returns the inner bytes, consuming the `SubjectBytes`.
    pub fn into_vec(self) -> Vec<u8> {
        self.0
    }
}

impl<'a> Arbitrary<'a> for SubjectIdBytes {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        // Generate bytes with length between 0 and SUBJ_ID_LEN
        let len = u.int_in_range(0..=SUBJ_ID_LEN)?;
        let mut bytes = vec![0u8; len];
        u.fill_buffer(&mut bytes)?;
        // Safe to unwrap since we ensure len <= SUBJ_ID_LEN
        Ok(Self::try_new(bytes).unwrap())
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

    mod subject_id_bytes {
        use super::*;

        proptest! {
            #[test]
            fn prop_accepts_valid_length(bytes in prop::collection::vec(any::<u8>(), 0..=SUBJ_ID_LEN)) {
                let result = SubjectIdBytes::try_new(bytes.clone());
                prop_assert!(result.is_ok());
                let sb = result.unwrap();
                prop_assert_eq!(sb.as_bytes(), &bytes[..]);
                prop_assert_eq!(sb.len(), bytes.len());
                prop_assert_eq!(sb.is_empty(), bytes.is_empty());
            }

            #[test]
            fn prop_rejects_too_long(
                bytes in prop::collection::vec(any::<u8>(), (SUBJ_ID_LEN + 1)..=(SUBJ_ID_LEN + 100))
            ) {
                let len = bytes.len();
                let result = SubjectIdBytes::try_new(bytes);
                prop_assert!(result.is_err());
                prop_assert!(matches!(result, Err(SubjectIdBytesError::TooLong(actual))
                    if actual == len));
            }

            #[test]
            fn prop_to_subject_id_preserves_and_pads(bytes in prop::collection::vec(any::<u8>(), 0..=SUBJ_ID_LEN)) {
                let sb = SubjectIdBytes::try_new(bytes.clone()).unwrap();
                let subject_id = sb.to_subject_id();
                let inner = subject_id.inner();

                let start = SUBJ_ID_LEN - bytes.len();

                // Original bytes should be preserved at the end
                prop_assert_eq!(&inner[start..], &bytes[..]);

                // Leading bytes should be zeros (padding)
                for &byte in &inner[..start] {
                    prop_assert_eq!(byte, 0);
                }

                // Total length should always be SUBJ_ID_LEN
                prop_assert_eq!(inner.len(), SUBJ_ID_LEN);
            }
        }
    }
}
