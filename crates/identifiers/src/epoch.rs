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

use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use const_hex as hex;
use serde::{Deserialize, Serialize};

use crate::{
    buf::Buf32,
    ol::{OLBlockCommitment, OLBlockId},
};

// TODO convert to u32
type RawEpoch = u64;

/// Commits to a particular epoch by the last block and slot.
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
pub struct EpochCommitment {
    epoch: RawEpoch,
    last_slot: u64,
    last_blkid: OLBlockId,
    // TODO convert to using OLBlockCommitment?
}

impl EpochCommitment {
    pub fn new(epoch: RawEpoch, last_slot: u64, last_blkid: OLBlockId) -> Self {
        Self {
            epoch,
            last_slot,
            last_blkid,
        }
    }

    /// Creates a new instance given the terminal block of an epoch and the
    /// epoch index.
    pub fn from_terminal(epoch: RawEpoch, block: OLBlockCommitment) -> Self {
        Self::new(epoch, block.slot(), *block.blkid())
    }

    /// Creates a "null" epoch with 0 slot, epoch 0, and zeroed blkid.
    pub fn null() -> Self {
        Self::new(0, 0, OLBlockId::from(Buf32::zero()))
    }

    pub fn epoch(&self) -> RawEpoch {
        self.epoch
    }

    pub fn last_slot(&self) -> u64 {
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
        // Show first 2 and last 2 bytes of block ID (4 hex chars each)
        let blkid_bytes = self.last_blkid().as_ref();
        let first_2 = &blkid_bytes[..2];
        let last_2 = &blkid_bytes[30..];

        let mut first_hex = [0u8; 4];
        let mut last_hex = [0u8; 4];
        hex::encode_to_slice(first_2, &mut first_hex)
            .expect("Failed to encode first 2 bytes to hex");
        hex::encode_to_slice(last_2, &mut last_hex).expect("Failed to encode last 2 bytes to hex");

        // SAFETY: we made sure of it
        write!(
            f,
            "{}[{}]@{}..{}",
            self.last_slot(),
            self.epoch(),
            unsafe { std::str::from_utf8_unchecked(&first_hex) },
            unsafe { std::str::from_utf8_unchecked(&last_hex) },
        )
    }
}

impl fmt::Debug for EpochCommitment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "EpochCommitment(epoch={}, last_slot={}, last_blkid={:?})",
            self.epoch(),
            self.last_slot(),
            self.last_blkid()
        )
    }
}
