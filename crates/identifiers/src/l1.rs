use std::{cmp::Ordering, fmt};

#[cfg(feature = "arbitrary")]
use arbitrary::Arbitrary;
#[cfg(feature = "borsh")]
use borsh::{BorshDeserialize, BorshSerialize};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};
#[cfg(feature = "ssz")]
use ssz_derive::{Decode, Encode};
#[cfg(feature = "codec")]
use strata_codec::Codec;

use crate::buf::{Buf32, RBuf32};

/// L1 block height (as a simple u32)
pub type L1Height = u32;

/// ID of an L1 block, usually the hash of its header.
///
/// Wraps [`RBuf32`] so that display and human-readable serde automatically
/// use Bitcoin's reversed byte order convention.
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Default)]
#[cfg_attr(feature = "ssz", derive(Encode, Decode))]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "arbitrary", derive(Arbitrary))]
#[cfg_attr(feature = "borsh", derive(BorshSerialize, BorshDeserialize))]
#[cfg_attr(feature = "codec", derive(Codec))]
pub struct L1BlockId(RBuf32);

// Debug, Display, From<RBuf32>, AsRef<[u8; 32]> via RBuf32 delegation.
crate::impl_buf_wrapper!(L1BlockId, RBuf32, 32);

impl From<Buf32> for L1BlockId {
    fn from(value: Buf32) -> Self {
        Self(RBuf32(value.0))
    }
}

impl From<L1BlockId> for Buf32 {
    fn from(value: L1BlockId) -> Self {
        Buf32(value.0.0)
    }
}

#[cfg(feature = "ssz")]
crate::impl_ssz_transparent_wrapper!(L1BlockId, RBuf32);

/// Witness transaction ID merkle root from a Bitcoin block.
///
/// This is the merkle root of all witness transaction IDs (wtxids) in a block.
/// Used instead of the regular transaction merkle root to include witness data
/// for complete transaction verification and malleability protection.
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Default)]
#[cfg_attr(feature = "ssz", derive(Encode, Decode))]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "arbitrary", derive(Arbitrary))]
#[cfg_attr(feature = "borsh", derive(BorshSerialize, BorshDeserialize))]
#[cfg_attr(feature = "codec", derive(Codec))]
pub struct WtxidsRoot(Buf32);

// Implement standard wrapper traits (Debug, Display, From, AsRef)
crate::impl_buf_wrapper!(WtxidsRoot, Buf32, 32);

#[cfg(feature = "ssz")]
crate::impl_ssz_transparent_wrapper!(WtxidsRoot, Buf32);

/// Commitment to an L1 block with height and ID.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Default)]
#[cfg_attr(feature = "ssz", derive(Encode, Decode))]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "arbitrary", derive(Arbitrary))]
#[cfg_attr(feature = "borsh", derive(BorshSerialize, BorshDeserialize))]
#[cfg_attr(feature = "codec", derive(Codec))]
#[cfg_attr(feature = "ssz", ssz(struct_behaviour = "container"))]
pub struct L1BlockCommitment {
    pub height: L1Height,
    pub blkid: L1BlockId,
}

#[cfg(feature = "ssz")]
crate::impl_ssz_fixed_container!(L1BlockCommitment, [height: L1Height, blkid: L1BlockId]);

impl fmt::Display for L1BlockCommitment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}", self.height, self.blkid)
    }
}

impl L1BlockCommitment {
    /// Create a new L1 block commitment.
    ///
    /// # Arguments
    /// * `height` - The block height
    /// * `blkid` - The block ID
    pub fn new(height: u32, blkid: L1BlockId) -> Self {
        Self { height, blkid }
    }

    /// Get the block height.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Get the block ID.
    pub fn blkid(&self) -> &L1BlockId {
        &self.blkid
    }
}

impl Ord for L1BlockCommitment {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.height(), self.blkid()).cmp(&(other.height(), other.blkid()))
    }
}

impl PartialOrd for L1BlockCommitment {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(all(test, feature = "ssz"))]
mod tests {
    use strata_test_utils_ssz::ssz_proptest;

    use super::*;
    use crate::test_utils::{buf32_strategy, l1_block_commitment_strategy};

    mod l1_block_id {
        use super::*;

        ssz_proptest!(
            L1BlockId,
            buf32_strategy(),
            transparent_wrapper_of(Buf32, from)
        );
    }

    mod l1_block_commitment {
        use super::*;

        ssz_proptest!(L1BlockCommitment, l1_block_commitment_strategy());
    }
}
