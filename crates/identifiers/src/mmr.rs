//! MMR (Merkle Mountain Range) identifier types.

use borsh::{BorshDeserialize, BorshSerialize};

use crate::AccountId;

pub type RawMmrId = Vec<u8>;

/// Identifier for a specific MMR instance in unified storage
///
/// Each variant represents a different MMR type, with optional scoping
/// within that type (e.g., per-account MMRs).
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Hash,
    BorshSerialize,
    BorshDeserialize,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
)]
pub enum MmrId {
    /// ASM manifest MMR (singleton, no account scope)
    Asm,
    /// Snark message inbox MMR (per-account scope)
    SnarkMsgInbox(AccountId),
}

impl MmrId {
    /// Serialize MmrId to bytes for use as database key
    ///
    /// Uses bincode with big-endian encoding to ensure lexicographical order because this will be
    /// used in the db key.
    pub fn to_bytes(&self) -> Vec<u8> {
        borsh::to_vec(&self).expect("MmrId serialization should not fail")
    }
}
