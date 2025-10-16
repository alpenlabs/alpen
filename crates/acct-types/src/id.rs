use std::fmt;

use int_enum::IntEnum;
use ssz_derive::{Decode, Encode};
use tree_hash::TreeHash;

use crate::impl_opaque_thin_wrapper;

type RawAccountId = [u8; 32];

/// Universal account identifier.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Encode, Decode)]
#[repr(transparent)]
#[ssz(struct_behaviour = "transparent")]
pub struct AccountId(RawAccountId);

// Manual TreeHash implementation for transparent wrapper
impl TreeHash for AccountId {
    fn tree_hash_type() -> tree_hash::TreeHashType {
        <[u8; 32] as TreeHash>::tree_hash_type()
    }

    fn tree_hash_packed_encoding(&self) -> tree_hash::PackedEncoding {
        <[u8; 32] as TreeHash>::tree_hash_packed_encoding(&self.0)
    }

    fn tree_hash_packing_factor() -> usize {
        <[u8; 32] as TreeHash>::tree_hash_packing_factor()
    }

    fn tree_hash_root(&self) -> tree_hash::Hash256 {
        <[u8; 32] as TreeHash>::tree_hash_root(&self.0)
    }
}

impl_opaque_thin_wrapper!(AccountId => RawAccountId);

type RawAccountSerial = u32;

/// Incrementally assigned account serial number.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Encode, Decode)]
#[repr(transparent)]
#[ssz(struct_behaviour = "transparent")]
pub struct AccountSerial(RawAccountSerial);

// Manual TreeHash implementation for transparent wrapper
impl TreeHash for AccountSerial {
    fn tree_hash_type() -> tree_hash::TreeHashType {
        <u32 as TreeHash>::tree_hash_type()
    }

    fn tree_hash_packed_encoding(&self) -> tree_hash::PackedEncoding {
        <u32 as TreeHash>::tree_hash_packed_encoding(&self.0)
    }

    fn tree_hash_packing_factor() -> usize {
        <u32 as TreeHash>::tree_hash_packing_factor()
    }

    fn tree_hash_root(&self) -> tree_hash::Hash256 {
        <u32 as TreeHash>::tree_hash_root(&self.0)
    }
}

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
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Encode, Decode)]
#[repr(transparent)]
#[ssz(struct_behaviour = "transparent")]
pub struct SubjectId(RawSubjectId);

// Manual TreeHash implementation for transparent wrapper
impl TreeHash for SubjectId {
    fn tree_hash_type() -> tree_hash::TreeHashType {
        <[u8; 32] as TreeHash>::tree_hash_type()
    }

    fn tree_hash_packed_encoding(&self) -> tree_hash::PackedEncoding {
        <[u8; 32] as TreeHash>::tree_hash_packed_encoding(&self.0)
    }

    fn tree_hash_packing_factor() -> usize {
        <[u8; 32] as TreeHash>::tree_hash_packing_factor()
    }

    fn tree_hash_root(&self) -> tree_hash::Hash256 {
        <[u8; 32] as TreeHash>::tree_hash_root(&self.0)
    }
}

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

// Manual SSZ implementations for C-like enum (serialize as u16)
impl ssz::Encode for AccountTypeId {
    fn is_ssz_fixed_len() -> bool {
        true
    }

    fn ssz_fixed_len() -> usize {
        2 // u16
    }

    fn ssz_append(&self, buf: &mut Vec<u8>) {
        (*self as u16).ssz_append(buf);
    }

    fn ssz_bytes_len(&self) -> usize {
        2
    }
}

impl ssz::Decode for AccountTypeId {
    fn is_ssz_fixed_len() -> bool {
        true
    }

    fn ssz_fixed_len() -> usize {
        2
    }

    fn from_ssz_bytes(bytes: &[u8]) -> Result<Self, ssz::DecodeError> {
        let val = u16::from_ssz_bytes(bytes)?;
        Self::try_from(val).map_err(|_| {
            ssz::DecodeError::BytesInvalid(format!("Invalid AccountTypeId discriminant: {}", val))
        })
    }
}

impl TreeHash for AccountTypeId {
    fn tree_hash_type() -> tree_hash::TreeHashType {
        <u16 as TreeHash>::tree_hash_type()
    }

    fn tree_hash_packed_encoding(&self) -> tree_hash::PackedEncoding {
        <u16 as TreeHash>::tree_hash_packed_encoding(&(*self as u16))
    }

    fn tree_hash_packing_factor() -> usize {
        <u16 as TreeHash>::tree_hash_packing_factor()
    }

    fn tree_hash_root(&self) -> tree_hash::Hash256 {
        <u16 as TreeHash>::tree_hash_root(&(*self as u16))
    }
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
    use ssz::{Decode, Encode};
    use tree_hash::TreeHash;

    use super::*;

    #[test]
    fn test_account_id_ssz_roundtrip() {
        let id = AccountId::new([42u8; 32]);
        let encoded = id.as_ssz_bytes();
        let decoded = AccountId::from_ssz_bytes(&encoded).unwrap();
        assert_eq!(id, decoded);
    }

    #[test]
    fn test_account_serial_ssz_roundtrip() {
        let serial = AccountSerial::new(12345);
        let encoded = serial.as_ssz_bytes();
        let decoded = AccountSerial::from_ssz_bytes(&encoded).unwrap();
        assert_eq!(serial, decoded);
    }

    #[test]
    fn test_account_serial_tree_hash() {
        let serial = AccountSerial::new(100);
        let hash = serial.tree_hash_root();
        // Should produce same hash as underlying u32
        assert_eq!(hash, <u32 as TreeHash>::tree_hash_root(&100u32));
    }

    #[test]
    fn test_subject_id_ssz_roundtrip() {
        let id = SubjectId::new([99u8; 32]);
        let encoded = id.as_ssz_bytes();
        let decoded = SubjectId::from_ssz_bytes(&encoded).unwrap();
        assert_eq!(id, decoded);
    }

    #[test]
    fn test_account_type_id_ssz_roundtrip() {
        let empty = AccountTypeId::Empty;
        let encoded = empty.as_ssz_bytes();
        let decoded = AccountTypeId::from_ssz_bytes(&encoded).unwrap();
        assert_eq!(empty, decoded);

        let snark = AccountTypeId::Snark;
        let encoded = snark.as_ssz_bytes();
        let decoded = AccountTypeId::from_ssz_bytes(&encoded).unwrap();
        assert_eq!(snark, decoded);
    }

    #[test]
    fn test_account_type_id_tree_hash() {
        let empty = AccountTypeId::Empty;
        let hash = empty.tree_hash_root();
        // Should produce same hash as u16 value 0
        assert_eq!(hash, <u16 as TreeHash>::tree_hash_root(&0u16));

        let snark = AccountTypeId::Snark;
        let hash = snark.tree_hash_root();
        // Should produce same hash as u16 value 1
        assert_eq!(hash, <u16 as TreeHash>::tree_hash_root(&1u16));
    }

    #[test]
    fn test_account_type_id_invalid_discriminant() {
        // Try to decode an invalid discriminant
        let invalid_bytes = 99u16.as_ssz_bytes();
        let result = AccountTypeId::from_ssz_bytes(&invalid_bytes);
        assert!(result.is_err());
    }
}
