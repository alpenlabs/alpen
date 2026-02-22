use std::{cmp::Ordering, fmt};

use arbitrary::{Arbitrary, Result as ArbitraryResult, Unstructured};
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use ssz_derive::{Decode, Encode};
use strata_codec::{Codec, CodecError, Decoder, Encoder};

use crate::{
    buf::{Buf32, RBuf32},
    ssz_generated::ssz::commitments::L1BlockCommitment,
};

/// The bitcoin block height
pub type BitcoinBlockHeight = u64;

/// L1 block height (as a simple u32)
pub type L1Height = u32;

/// ID of an L1 block, usually the hash of its header.
///
/// Wraps [`RBuf32`] so that display and human-readable serde automatically
/// use Bitcoin's reversed byte order convention.
#[derive(
    Copy,
    Clone,
    Eq,
    PartialEq,
    Ord,
    PartialOrd,
    Hash,
    Default,
    Arbitrary,
    BorshSerialize,
    BorshDeserialize,
    Serialize,
    Deserialize,
    Encode,
    Decode,
)]
pub struct L1BlockId(RBuf32);

// Debug, Display, From<RBuf32>, AsRef<[u8; 32]>, and Codec via RBuf32 delegation.
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

// Manual TreeHash implementation for transparent wrapper
crate::impl_ssz_transparent_wrapper!(L1BlockId, RBuf32, 32);

/// Witness transaction ID merkle root from a Bitcoin block.
///
/// This is the merkle root of all witness transaction IDs (wtxids) in a block.
/// Used instead of the regular transaction merkle root to include witness data
/// for complete transaction verification and malleability protection.
#[derive(
    Copy,
    Clone,
    Eq,
    PartialEq,
    Ord,
    PartialOrd,
    Hash,
    Default,
    Arbitrary,
    BorshSerialize,
    BorshDeserialize,
    Deserialize,
    Serialize,
    Encode,
    Decode,
)]
pub struct WtxidsRoot(Buf32);

// Implement standard wrapper traits (Debug, Display, From, AsRef, Codec)
crate::impl_buf_wrapper!(WtxidsRoot, Buf32, 32);

// Manual TreeHash implementation for transparent wrapper
crate::impl_ssz_transparent_buf32_wrapper!(WtxidsRoot);

// Use macro to generate Borsh implementations via SSZ (fixed-size, no length prefix)
crate::impl_borsh_via_ssz_fixed!(L1BlockCommitment);

impl Codec for L1BlockCommitment {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        // Encode height as u64 for consistency
        let height_u64 = self.height as u64;

        height_u64.encode(enc)?;
        self.blkid.encode(enc)?;
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let height_u64 = u64::decode(dec)?;
        let height = height_u64 as u32;

        let blkid = L1BlockId::decode(dec)?;
        Ok(Self { height, blkid })
    }
}

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

    /// Create a new L1 block commitment from a u64 height.
    ///
    /// Returns `None` if the height is invalid (greater than u32::MAX).
    pub fn from_height_u64(height: u64, blkid: L1BlockId) -> Option<Self> {
        Some(Self {
            height: height as u32,
            blkid,
        })
    }

    pub fn height_u32(&self) -> u32 {
        self.height
    }

    /// Get the block height as u64 for compatibility.
    pub fn height_u64(&self) -> u64 {
        self.height as u64
    }

    /// Get the block ID.
    pub fn blkid(&self) -> &L1BlockId {
        &self.blkid
    }
}

impl Arbitrary<'_> for L1BlockCommitment {
    fn arbitrary(u: &mut Unstructured<'_>) -> ArbitraryResult<Self> {
        let height = u32::arbitrary(u)?;
        let blkid = L1BlockId::arbitrary(u)?;
        Ok(Self { height, blkid })
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

#[cfg(test)]
mod tests {
    use ssz::{Decode, Encode};
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

        #[test]
        fn test_zero_ssz() {
            let zero = L1BlockId::from(Buf32::zero());
            let encoded = zero.as_ssz_bytes();
            let decoded = L1BlockId::from_ssz_bytes(&encoded).unwrap();
            assert_eq!(zero, decoded);
        }
    }

    mod l1_block_commitment {
        use super::*;

        ssz_proptest!(L1BlockCommitment, l1_block_commitment_strategy());

        #[test]
        fn test_zero_ssz() {
            let commitment = L1BlockCommitment::new(0, L1BlockId::from(Buf32::zero()));

            let encoded = commitment.as_ssz_bytes();
            let decoded = L1BlockCommitment::from_ssz_bytes(&encoded).unwrap();
            assert_eq!(commitment.height_u64(), decoded.height_u64());
            assert_eq!(commitment.blkid(), decoded.blkid());
        }
    }
}
