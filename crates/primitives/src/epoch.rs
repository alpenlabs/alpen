//! Types relating to epoch bookkeeping.
//!
//! An epoch of a range of sequential blocks defined by the terminal block of
//! the epoch going back to (but not including) the terminal block of a previous
//! epoch.  This uniquely identifies the epoch's final state indirectly,
//! although it's possible for conflicting epochs with different terminal blocks
//! to exist in theory, depending on the consensus algorithm.
//!
//! Epochs are *usually* always the same number of slots, but we're not
//! guaranteeing this yet, so we always include both the epoch number and slot
//! number of the terminal block.
//!
//! We also have a sentinel "null" epoch used to refer to the "finalized epoch"
//! as of the genesis block.

use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};

use crate::{
    buf::Buf32,
    l2::{L2BlockCommitment, L2BlockId},
};

/// Commits to a particular epoch by the last block and slot.
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
pub struct EpochCommitment {
    epoch: u64,
    last_slot: u64,
    last_blkid: L2BlockId,
}

impl EpochCommitment {
    #[expect(clippy::missing_const_for_fn)]
    pub fn new(epoch: u64, last_slot: u64, last_blkid: L2BlockId) -> Self {
        Self {
            epoch,
            last_slot,
            last_blkid,
        }
    }

    /// Creates a new instance given the terminal block of an epoch and the
    /// epoch index.
    pub fn from_terminal(epoch: u64, block: L2BlockCommitment) -> Self {
        Self::new(epoch, block.slot(), *block.blkid())
    }

    /// Creates a "null" epoch with 0 slot, epoch 0, and zeroed blkid.
    pub fn null() -> Self {
        Self::new(0, 0, L2BlockId::from(Buf32::zero()))
    }

    pub const fn epoch(&self) -> u64 {
        self.epoch
    }

    pub const fn last_slot(&self) -> u64 {
        self.last_slot
    }

    pub const fn last_blkid(&self) -> &L2BlockId {
        &self.last_blkid
    }

    /// Returns a [`L2BlockCommitment`] for the final block of the epoch.
    pub fn to_block_commitment(&self) -> L2BlockCommitment {
        L2BlockCommitment::new(self.last_slot, self.last_blkid)
    }

    /// Returns if the terminal blkid is zero.  This signifies a special case
    /// for the genesis epoch (0) before the it is completed.
    pub fn is_null(&self) -> bool {
        Buf32::from(self.last_blkid).is_zero()
    }
}
