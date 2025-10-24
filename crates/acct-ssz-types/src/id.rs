// SSZ type definitions for account identifiers.
// Types defined here match the pythonic schema in `schemas/acct-types.ssz`.

use int_enum::IntEnum;
use ssz_derive::{Decode, Encode};

/// Unique identifier for an account (32-byte hash)
/// Schema: class AccountId(Bytes32)
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Encode, Decode)]
#[repr(transparent)]
#[ssz(struct_behaviour = "transparent")]
pub struct AccountId(pub [u8; 32]);

// Manual TreeHash implementation for transparent wrapper
impl tree_hash::TreeHash for AccountId {
    fn tree_hash_type() -> tree_hash::TreeHashType {
        <[u8; 32] as tree_hash::TreeHash>::tree_hash_type()
    }

    fn tree_hash_packed_encoding(&self) -> tree_hash::PackedEncoding {
        <[u8; 32] as tree_hash::TreeHash>::tree_hash_packed_encoding(&self.0)
    }

    fn tree_hash_packing_factor() -> usize {
        <[u8; 32] as tree_hash::TreeHash>::tree_hash_packing_factor()
    }

    fn tree_hash_root(&self) -> tree_hash::Hash256 {
        <[u8; 32] as tree_hash::TreeHash>::tree_hash_root(&self.0)
    }
}

/// Unique identifier for a subject within an execution environment
/// Schema: class SubjectId(Bytes32)
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Encode, Decode)]
#[repr(transparent)]
#[ssz(struct_behaviour = "transparent")]
pub struct SubjectId(pub [u8; 32]);

// Manual TreeHash implementation for transparent wrapper
impl tree_hash::TreeHash for SubjectId {
    fn tree_hash_type() -> tree_hash::TreeHashType {
        <[u8; 32] as tree_hash::TreeHash>::tree_hash_type()
    }

    fn tree_hash_packed_encoding(&self) -> tree_hash::PackedEncoding {
        <[u8; 32] as tree_hash::TreeHash>::tree_hash_packed_encoding(&self.0)
    }

    fn tree_hash_packing_factor() -> usize {
        <[u8; 32] as tree_hash::TreeHash>::tree_hash_packing_factor()
    }

    fn tree_hash_root(&self) -> tree_hash::Hash256 {
        <[u8; 32] as tree_hash::TreeHash>::tree_hash_root(&self.0)
    }
}

/// Serial number for account state transitions
/// Schema: class AccountSerial(uint32)
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Encode, Decode)]
#[repr(transparent)]
#[ssz(struct_behaviour = "transparent")]
pub struct AccountSerial(pub u32);

// Manual TreeHash implementation for transparent wrapper
impl tree_hash::TreeHash for AccountSerial {
    fn tree_hash_type() -> tree_hash::TreeHashType {
        <u32 as tree_hash::TreeHash>::tree_hash_type()
    }

    fn tree_hash_packed_encoding(&self) -> tree_hash::PackedEncoding {
        <u32 as tree_hash::TreeHash>::tree_hash_packed_encoding(&self.0)
    }

    fn tree_hash_packing_factor() -> usize {
        <u32 as tree_hash::TreeHash>::tree_hash_packing_factor()
    }

    fn tree_hash_root(&self) -> tree_hash::Hash256 {
        <u32 as tree_hash::TreeHash>::tree_hash_root(&self.0)
    }
}

/// Raw primitive version of an account type ID
pub type RawAccountTypeId = u16;

/// Distinguishes between account types
/// Schema: class AccountTypeId(uint16) - Note: This is an enum in Rust
#[repr(u16)]
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, IntEnum)]
pub enum AccountTypeId {
    /// "Inert" account type for a stub that exists but does nothing, but store balance
    Empty = 0,
    /// Snark accounts
    Snark = 1,
}

// Manual SSZ encode/decode for AccountTypeId
impl ssz::Encode for AccountTypeId {
    fn is_ssz_fixed_len() -> bool {
        true
    }

    fn ssz_fixed_len() -> usize {
        2
    }

    fn ssz_bytes_len(&self) -> usize {
        2
    }

    fn ssz_append(&self, buf: &mut Vec<u8>) {
        (*self as u16).ssz_append(buf);
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
        let raw = u16::from_ssz_bytes(bytes)?;
        AccountTypeId::try_from(raw).map_err(|_| {
            ssz::DecodeError::BytesInvalid(format!("Invalid AccountTypeId value: {}", raw))
        })
    }
}

// Manual TreeHash for AccountTypeId
impl tree_hash::TreeHash for AccountTypeId {
    fn tree_hash_type() -> tree_hash::TreeHashType {
        tree_hash::TreeHashType::Basic
    }

    fn tree_hash_packed_encoding(&self) -> tree_hash::PackedEncoding {
        <u16 as tree_hash::TreeHash>::tree_hash_packed_encoding(&(*self as u16))
    }

    fn tree_hash_packing_factor() -> usize {
        <u16 as tree_hash::TreeHash>::tree_hash_packing_factor()
    }

    fn tree_hash_root(&self) -> tree_hash::Hash256 {
        <u16 as tree_hash::TreeHash>::tree_hash_root(&(*self as u16))
    }
}
