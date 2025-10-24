//! Account identifier types.
//!
//! Type definitions are included from `strata-acct-ssz-types` and extended with
//! business logic methods here.

use std::fmt;

// Include SSZ type definitions from acct-ssz-types
// This brings in: AccountId, SubjectId, AccountSerial, AccountTypeId, RawAccountTypeId
// along with their Encode, Decode, TreeHash implementations
include!("../../acct-ssz-types/src/id.rs");

// Codec implementations for wrapper types (provides new() and inner() methods)
use crate::impl_opaque_thin_wrapper;
impl_opaque_thin_wrapper!(AccountId => [u8; 32]);
impl_opaque_thin_wrapper!(SubjectId => [u8; 32]);
impl_opaque_thin_wrapper!(AccountSerial => u32);

// Business logic: additional methods for AccountSerial
impl AccountSerial {
    /// Increments the serial number.
    ///
    /// # Panics
    /// Panics if the serial number is at MAX.
    pub fn incr(self) -> AccountSerial {
        if *self.inner() == u32::MAX {
            panic!("acctsys: reached max serial number");
        }
        AccountSerial::new(self.inner() + 1)
    }
}

// Business logic: Display implementation for AccountTypeId
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
        let id = AccountId([42u8; 32]);
        let encoded = id.as_ssz_bytes();
        let decoded = AccountId::from_ssz_bytes(&encoded).unwrap();
        assert_eq!(id, decoded);
    }

    #[test]
    fn test_account_id_tree_hash() {
        let id1 = AccountId([1u8; 32]);
        let id2 = AccountId([1u8; 32]);
        assert_eq!(id1.tree_hash_root(), id2.tree_hash_root());
    }

    #[test]
    fn test_subject_id_ssz_roundtrip() {
        let id = SubjectId([99u8; 32]);
        let encoded = id.as_ssz_bytes();
        let decoded = SubjectId::from_ssz_bytes(&encoded).unwrap();
        assert_eq!(id, decoded);
    }

    #[test]
    fn test_account_serial_ssz_roundtrip() {
        let serial = AccountSerial(12345);
        let encoded = serial.as_ssz_bytes();
        let decoded = AccountSerial::from_ssz_bytes(&encoded).unwrap();
        assert_eq!(serial, decoded);
    }

    #[test]
    fn test_account_serial_incr() {
        let serial = AccountSerial(5);
        assert_eq!(serial.incr().inner(), &6);
    }

    #[test]
    fn test_account_type_id_ssz_roundtrip() {
        let ty = AccountTypeId::Snark;
        let encoded = ty.as_ssz_bytes();
        let decoded = AccountTypeId::from_ssz_bytes(&encoded).unwrap();
        assert_eq!(ty, decoded);
    }

    #[test]
    fn test_account_type_id_display() {
        assert_eq!(format!("{}", AccountTypeId::Empty), "empty");
        assert_eq!(format!("{}", AccountTypeId::Snark), "snark");
    }
}
