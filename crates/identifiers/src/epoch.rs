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

use std::{cmp, fmt, str};

use const_hex as hex;
use strata_codec::{Codec, CodecError, Decoder, Encoder};

use crate::{
    Epoch, Slot,
    buf::Buf32,
    ol::OLBlockId,
    ssz_generated::ssz::commitments::{EpochCommitment, OLBlockCommitment},
};

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

impl Codec for EpochCommitment {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        self.epoch.encode(enc)?;
        self.last_slot.encode(enc)?;
        self.last_blkid.encode(enc)?;
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let epoch = u32::decode(dec)?;
        let last_slot = u64::decode(dec)?;
        let last_blkid = OLBlockId::decode(dec)?;
        Ok(Self {
            epoch,
            last_slot,
            last_blkid,
        })
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

        // SAFETY: hex always encodes 2->4 bytes
        write!(
            f,
            "{}[{}]@{}..{}",
            self.last_slot(),
            self.epoch(),
            unsafe { str::from_utf8_unchecked(&first_hex) },
            unsafe { str::from_utf8_unchecked(&last_hex) },
        )
    }
}

impl<'a> arbitrary::Arbitrary<'a> for EpochCommitment {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        Ok(Self {
            epoch: u.arbitrary()?,
            last_slot: u.arbitrary()?,
            last_blkid: u.arbitrary()?,
        })
    }
}

impl Ord for EpochCommitment {
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        (self.epoch, self.last_slot, &self.last_blkid).cmp(&(
            other.epoch,
            other.last_slot,
            &other.last_blkid,
        ))
    }
}

impl PartialOrd for EpochCommitment {
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
mod tests {
    use ssz::{Decode, Encode};
    use strata_test_utils_ssz::ssz_proptest;

    use super::*;
    use crate::test_utils::epoch_commitment_strategy;

    mod epoch_commitment {
        use super::*;

        ssz_proptest!(EpochCommitment, epoch_commitment_strategy());

        #[test]
        fn test_zero_ssz() {
            let commitment = EpochCommitment::null();
            let encoded = commitment.as_ssz_bytes();
            let decoded = EpochCommitment::from_ssz_bytes(&encoded).unwrap();
            assert_eq!(commitment.epoch(), decoded.epoch());
            assert_eq!(commitment.last_slot(), decoded.last_slot());
            assert_eq!(commitment.last_blkid(), decoded.last_blkid());
        }
    }
}
