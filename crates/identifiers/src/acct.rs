use std::{
    fmt,
    hash::{Hash, Hasher},
    io::{self, Read, Write},
    mem,
};

use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use int_enum::IntEnum;
use serde::{Deserialize, Serialize};
use ssz::view;
use ssz_derive::{Decode, Encode};
use strata_codec::{Codec, CodecError, VARINT_MAX, Varint};
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
    Arbitrary,
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

type RawAccountSerial = Varint;

/// Size of AccountSerial's SSZ/Borsh representation (u32) in bytes.
const RAW_ACCOUNT_SERIAL_LEN: usize = mem::size_of::<u32>();

/// Incrementally assigned account serial number.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Codec)]
pub struct AccountSerial(RawAccountSerial);

impl AccountSerial {
    /// Returns the zero serial.
    pub fn zero() -> AccountSerial {
        AccountSerial::new(0).expect("acctsys: zero is within varint bounds")
    }

    /// Returns the one serial.
    pub fn one() -> AccountSerial {
        AccountSerial::new(1).expect("acctsys: one is within varint bounds")
    }

    /// Creates a serial from a raw u32 value.
    pub fn new(value: u32) -> Option<Self> {
        RawAccountSerial::new(value).map(Self)
    }

    /// Returns the raw varint representation.
    pub fn inner(&self) -> &RawAccountSerial {
        &self.0
    }

    /// Consumes the serial and returns the raw varint representation.
    pub fn into_inner(self) -> RawAccountSerial {
        self.0
    }

    /// Returns the numeric serial value.
    pub fn value(&self) -> u32 {
        self.0.inner()
    }

    /// Creates a serial for one of the reserved accounts.
    ///
    /// # Panics
    ///
    /// If the ID provided is outside the valid range.
    pub fn reserved(b: u8) -> Self {
        let value = b as u32;
        assert!(
            value < SYSTEM_RESERVED_ACCTS,
            "acct: out of bounds reserved serial"
        );
        AccountSerial::new(value).expect("acct: reserved serial within varint bounds")
    }

    pub fn incr(self) -> AccountSerial {
        if self.value() == VARINT_MAX {
            panic!("acctsys: reached max serial number");
        }

        AccountSerial::new(self.value() + 1).expect("acctsys: serial increment within bounds")
    }

    pub fn is_reserved(&self) -> bool {
        self.value() < SYSTEM_RESERVED_ACCTS
    }
}

impl fmt::Display for AccountSerial {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "serial:{}", self.value())
    }
}

impl From<RawAccountSerial> for AccountSerial {
    fn from(value: RawAccountSerial) -> AccountSerial {
        AccountSerial(value)
    }
}

impl From<AccountSerial> for RawAccountSerial {
    fn from(value: AccountSerial) -> RawAccountSerial {
        value.0
    }
}

impl TryFrom<u32> for AccountSerial {
    type Error = CodecError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        AccountSerial::new(value).ok_or(CodecError::OobInteger)
    }
}

impl From<AccountSerial> for u32 {
    fn from(value: AccountSerial) -> u32 {
        value.value()
    }
}

impl Hash for AccountSerial {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.value().hash(state);
    }
}

impl<'a> Arbitrary<'a> for AccountSerial {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        let value = u.int_in_range(0..=VARINT_MAX)?;
        Ok(AccountSerial::new(value).expect("acctsys: arbitrary serial within bounds"))
    }
}

impl ssz::Encode for AccountSerial {
    fn is_ssz_fixed_len() -> bool {
        true
    }

    fn ssz_fixed_len() -> usize {
        RAW_ACCOUNT_SERIAL_LEN
    }

    fn ssz_append(&self, buf: &mut Vec<u8>) {
        self.value().ssz_append(buf);
    }

    fn ssz_bytes_len(&self) -> usize {
        Self::ssz_fixed_len()
    }
}

impl ssz::Decode for AccountSerial {
    fn is_ssz_fixed_len() -> bool {
        true
    }

    fn ssz_fixed_len() -> usize {
        RAW_ACCOUNT_SERIAL_LEN
    }

    fn from_ssz_bytes(bytes: &[u8]) -> Result<Self, ssz::DecodeError> {
        if bytes.len() != Self::ssz_fixed_len() {
            return Err(ssz::DecodeError::InvalidByteLength {
                len: bytes.len(),
                expected: Self::ssz_fixed_len(),
            });
        }

        let value = u32::from_ssz_bytes(bytes)?;
        AccountSerial::new(value).ok_or_else(|| {
            ssz::DecodeError::BytesInvalid(format!(
                "account serial {value} exceeds varint max {VARINT_MAX}"
            ))
        })
    }
}

impl<'a> view::DecodeView<'a> for AccountSerial {
    fn from_ssz_bytes(bytes: &'a [u8]) -> Result<Self, ssz::DecodeError> {
        <Self as ssz::Decode>::from_ssz_bytes(bytes)
    }
}

impl view::SszTypeInfo for AccountSerial {
    fn is_ssz_fixed_len() -> bool {
        true
    }

    fn ssz_fixed_len() -> usize {
        RAW_ACCOUNT_SERIAL_LEN
    }
}

impl<H: tree_hash::TreeHashDigest> tree_hash::TreeHash<H> for AccountSerial {
    fn tree_hash_type() -> tree_hash::TreeHashType {
        <u32 as tree_hash::TreeHash<H>>::tree_hash_type()
    }

    fn tree_hash_packed_encoding(&self) -> tree_hash::PackedEncoding {
        <u32 as tree_hash::TreeHash<H>>::tree_hash_packed_encoding(&self.value())
    }

    fn tree_hash_packing_factor() -> usize {
        <u32 as tree_hash::TreeHash<H>>::tree_hash_packing_factor()
    }

    fn tree_hash_root(&self) -> H::Output {
        <u32 as tree_hash::TreeHash<H>>::tree_hash_root(&self.value())
    }
}

impl BorshSerialize for AccountSerial {
    fn serialize<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        BorshSerialize::serialize(&self.value(), writer)
    }
}

impl BorshDeserialize for AccountSerial {
    fn deserialize_reader<R: Read>(reader: &mut R) -> io::Result<Self> {
        let value = u32::deserialize_reader(reader)?;
        AccountSerial::new(value).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("account serial {value} exceeds varint max {VARINT_MAX}"),
            )
        })
    }
}

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
    use strata_codec::VARINT_MAX;
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
        use tree_hash::{Sha256Hasher, TreeHash};

        use super::*;

        ssz_proptest!(
            AccountSerial,
            (0..=VARINT_MAX).prop_map(|value| {
                AccountSerial::try_from(value).expect("serial is within varint bounds")
            })
        );

        proptest! {
            #[test]
            fn tree_hash_transparent(value in 0..=VARINT_MAX) {
                let serial = AccountSerial::try_from(value)
                    .expect("serial is within varint bounds");
                let wrapper_hash = <AccountSerial as TreeHash<Sha256Hasher>>::tree_hash_root(&serial);
                let inner_hash = <u32 as TreeHash<Sha256Hasher>>::tree_hash_root(&value);
                prop_assert_eq!(wrapper_hash, inner_hash);
            }
        }

        #[test]
        fn test_zero_ssz() {
            let zero = AccountSerial::new(0).expect("serial is within varint bounds");
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
