use std::fmt;

use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use const_hex as hex;
use serde::{Deserialize, Serialize};

use crate::buf::Buf32;

/// ID of an OL (Optimistic Ledger) block, usually the hash of its root header.
#[derive(
    Copy,
    Clone,
    Eq,
    Default,
    PartialEq,
    Ord,
    PartialOrd,
    Hash,
    Arbitrary,
    BorshSerialize,
    BorshDeserialize,
    Serialize,
    Deserialize,
)]
pub struct OLBlockId(Buf32);

impl_buf_wrapper!(OLBlockId, Buf32, 32);

impl OLBlockId {
    /// Returns a dummy blkid that is all zeroes.
    pub fn null() -> Self {
        Self::from(Buf32::zero())
    }

    /// Checks to see if this is the dummy "zero" blkid.
    pub fn is_null(&self) -> bool {
        self.0.is_zero()
    }
}

/// Alias for backward compatibility
pub type L2BlockId = OLBlockId;

/// Commits to a specific block at some slot.
#[derive(
    Copy,
    Clone,
    Eq,
    PartialEq,
    Ord,
    PartialOrd,
    Hash,
    Arbitrary,
    BorshDeserialize,
    BorshSerialize,
    Deserialize,
    Serialize,
)]
pub struct OLBlockCommitment {
    slot: u64,
    blkid: OLBlockId,
}

impl OLBlockCommitment {
    pub fn new(slot: u64, blkid: OLBlockId) -> Self {
        Self { slot, blkid }
    }

    pub fn null() -> Self {
        Self::new(0, OLBlockId::from(Buf32::zero()))
    }

    pub fn slot(&self) -> u64 {
        self.slot
    }

    pub fn blkid(&self) -> &OLBlockId {
        &self.blkid
    }

    pub fn is_null(&self) -> bool {
        self.slot == 0 && self.blkid.0.is_zero()
    }
}

impl fmt::Display for OLBlockCommitment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Show first 2 and last 2 bytes of block ID (4 hex chars each)
        let blkid_bytes = self.blkid.as_ref();
        let first_2 = &blkid_bytes[..2];
        let last_2 = &blkid_bytes[30..];

        let mut first_hex = [0u8; 4];
        let mut last_hex = [0u8; 4];
        hex::encode_to_slice(first_2, &mut first_hex)
            .expect("Failed to encode first 2 bytes to hex");
        hex::encode_to_slice(last_2, &mut last_hex).expect("Failed to encode last 2 bytes to hex");

        write!(
            f,
            "{}@{}..{}",
            self.slot,
            std::str::from_utf8(&first_hex)
                .expect("Failed to convert first hex bytes to UTF-8 string"),
            std::str::from_utf8(&last_hex).expect("Failed to convert last hex bytes to UTF-8 string")
        )
    }
}

impl fmt::Debug for OLBlockCommitment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "OLBlockCommitment(slot={}, blkid={:?})",
            self.slot, self.blkid
        )
    }
}

/// Alias for backward compatibility
pub type L2BlockCommitment = OLBlockCommitment;
