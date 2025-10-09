//! Execution block commitments.

use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};

use crate::buf::Buf32;

/// A commitment to an execution block for some arbitrary execution chain at a
/// particular slot.
///
/// Also permits a concept of a "null" block.
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
pub struct ExecBlockCommitment {
    // TODO rename slot to number?
    slot: u64,
    blkid: Buf32,
}

impl ExecBlockCommitment {
    pub fn new(slot: u64, blkid: Buf32) -> Self {
        Self { slot, blkid }
    }

    pub fn null() -> Self {
        Self::new(0, Buf32::zero())
    }

    pub fn slot(&self) -> u64 {
        self.slot
    }

    pub fn blkid(&self) -> &Buf32 {
        &self.blkid
    }

    pub fn is_null(&self) -> bool {
        self.slot == 0 && self.blkid().is_zero()
    }
}
