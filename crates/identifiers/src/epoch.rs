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

use std::fmt;

#[cfg(feature = "borsh")]
use borsh::{BorshDeserialize, BorshSerialize};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};
#[cfg(feature = "ssz")]
use ssz_derive::{Decode, Encode};
#[cfg(feature = "codec")]
use strata_codec::Codec;

use crate::{
    Epoch, Slot,
    buf::Buf32,
    ol::{OLBlockCommitment, OLBlockId},
};

/// Commitment to a particular epoch by the last block and slot.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Default)]
#[cfg_attr(feature = "ssz", derive(Encode, Decode))]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
#[cfg_attr(feature = "borsh", derive(BorshSerialize, BorshDeserialize))]
#[cfg_attr(feature = "codec", derive(Codec))]
#[cfg_attr(feature = "ssz", ssz(struct_behaviour = "container"))]
pub struct EpochCommitment {
    pub epoch: Epoch,
    pub last_slot: Slot,
    pub last_blkid: OLBlockId,
}

#[cfg(feature = "ssz")]
crate::impl_ssz_fixed_container!(EpochCommitment, [epoch: Epoch, last_slot: Slot, last_blkid: OLBlockId]);

impl EpochCommitment {
    pub fn new(epoch: Epoch, last_slot: Slot, last_blkid: OLBlockId) -> Self {
        Self {
            epoch,
            last_slot,
            last_blkid,
        }
    }

    /// Creates a new instance given the terminal block of an epoch and the
    /// epoch index.
    pub fn from_terminal(epoch: Epoch, block: OLBlockCommitment) -> Self {
        Self::new(epoch, block.slot(), *block.blkid())
    }

    /// Creates a "null" epoch with 0 slot, epoch 0, and zeroed blkid.
    pub fn null() -> Self {
        Self::new(0, 0, OLBlockId::from(Buf32::zero()))
    }

    pub fn epoch(&self) -> Epoch {
        self.epoch
    }

    pub fn last_slot(&self) -> Slot {
        self.last_slot
    }

    pub fn last_blkid(&self) -> &OLBlockId {
        &self.last_blkid
    }

    /// Returns a [`OLBlockCommitment`] for the final block of the epoch.
    pub fn to_block_commitment(&self) -> OLBlockCommitment {
        OLBlockCommitment::new(self.last_slot, self.last_blkid)
    }

    /// Returns if the terminal blkid is zero.  This signifies a special case
    /// for the genesis epoch (0) before the it is completed.
    pub fn is_null(&self) -> bool {
        Buf32::from(self.last_blkid).is_zero()
    }
}

impl fmt::Display for EpochCommitment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}[{}]@{}",
            self.last_slot(),
            self.epoch(),
            self.last_blkid(),
        )
    }
}

#[cfg(all(test, feature = "ssz"))]
mod tests {
    use strata_test_utils_ssz::ssz_proptest;

    use super::*;
    use crate::test_utils::epoch_commitment_strategy;

    mod epoch_commitment {
        use super::*;

        ssz_proptest!(EpochCommitment, epoch_commitment_strategy());
    }
}
