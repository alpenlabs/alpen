use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};

use crate::{buf::Buf32, impl_buf_wrapper};

/// ID of an L2 block, usually the hash of its root header.
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
pub struct L2BlockId(Buf32);

impl_buf_wrapper!(L2BlockId, Buf32, 32);

impl L2BlockId {
    /// Returns a dummy blkid that is all zeroes.
    pub fn null() -> Self {
        Self::from(Buf32::zero())
    }

    /// Checks to see if this is the dummy "zero" blkid.
    pub fn is_null(&self) -> bool {
        self.0.is_zero()
    }
}

/// Commits to a specific block at some slot.
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
    BorshDeserialize,
    BorshSerialize,
    Deserialize,
    Serialize,
)]
pub struct L2BlockCommitment {
    slot: u64,
    blkid: L2BlockId,
}

impl L2BlockCommitment {
    pub fn new(slot: u64, blkid: L2BlockId) -> Self {
        Self { slot, blkid }
    }

    pub fn null() -> Self {
        Self::new(0, L2BlockId::from(Buf32::zero()))
    }

    pub const fn slot(&self) -> u64 {
        self.slot
    }

    pub const fn blkid(&self) -> &L2BlockId {
        &self.blkid
    }

    pub fn is_null(&self) -> bool {
        self.slot == 0 && self.blkid.0.is_zero()
    }
}
