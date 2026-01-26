use std::{cmp::Ordering, fmt, str};

use arbitrary::{Arbitrary, Result as ArbitraryResult, Unstructured};
use borsh::{BorshDeserialize, BorshSerialize};
use const_hex as hex;
use hex::encode_to_slice;
use serde::{Deserialize, Serialize};
use ssz_derive::{Decode, Encode};
use strata_codec::{Codec, CodecError, Decoder, Encoder};

use crate::buf::Buf32;
use crate::ssz_generated::ssz::commitments::L1BlockCommitment;

/// L1 block height (u32 in consensus format).
pub type L1Height = u32;

/// ID of an L1 block, usually the hash of its header.
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
pub struct L1BlockId(Buf32);

// Custom implementation without Debug/Display to avoid conflicts
impl From<Buf32> for L1BlockId {
    fn from(value: Buf32) -> Self {
        Self(value)
    }
}

impl From<L1BlockId> for Buf32 {
    fn from(value: L1BlockId) -> Self {
        value.0
    }
}

impl Codec for L1BlockId {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        self.0.encode(enc)
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let buf = Buf32::decode(dec)?;
        Ok(Self(buf))
    }
}

impl AsRef<[u8; 32]> for L1BlockId {
    fn as_ref(&self) -> &[u8; 32] {
        self.0.as_ref()
    }
}

// Manual TreeHash implementation for transparent wrapper
crate::impl_ssz_transparent_buf32_wrapper!(L1BlockId);

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

/// L1 Block commitment with block height and ID.
///
/// Height is stored as a `u32` in consensus format.
impl L1BlockCommitment {
    /// Create a new L1 block commitment.
    ///
    /// # Arguments
    /// * `height` - The block height
    /// * `blkid` - The block ID
    pub fn new(height: L1Height, blkid: L1BlockId) -> Self {
        Self { height, blkid }
    }

    /// Create a new L1 block commitment from a u64 height.
    ///
    /// Returns `None` if the height is greater than `u32::MAX`.
    pub fn from_height_u64(height: u64, blkid: L1BlockId) -> Option<Self> {
        let height = u32::try_from(height).ok()?;
        Some(Self { height, blkid })
    }

    /// Get the block height.
    pub fn height(&self) -> L1Height {
        self.height
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
        let height = u32::try_from(height_u64)
            .map_err(|_| CodecError::MalformedField("L1BlockCommitment.height"))?;
        let blkid = L1BlockId::decode(dec)?;
        Ok(Self { height, blkid })
    }
}

#[expect(
    clippy::derivable_impls,
    reason = "ssz-generated type; cannot add derives here"
)]
impl Default for L1BlockCommitment {
    fn default() -> Self {
        Self {
            height: 0,
            blkid: L1BlockId::default(),
        }
    }
}

impl fmt::Display for L1BlockCommitment {
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
            self.height,
            str::from_utf8(&first_hex)
                .expect("Failed to convert first 2 hex bytes to UTF-8 string"),
            str::from_utf8(&last_hex).expect("Failed to convert last 2 hex bytes to UTF-8 string")
        )
    }
}

// Use macro to generate Borsh implementations via SSZ (fixed-size, no length prefix)
crate::impl_borsh_via_ssz_fixed!(L1BlockCommitment);

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

// Custom debug implementation to print the block hash in little endian
impl fmt::Debug for L1BlockId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut bytes = self.0.0;
        bytes.reverse();
        let mut buf = [0u8; 64]; // 32 bytes * 2 for hex
        encode_to_slice(bytes, &mut buf).expect("buf: enc hex");
        // SAFETY: hex encoding always produces valid UTF-8
        let hex_str = unsafe { str::from_utf8_unchecked(&buf) };
        f.write_str(hex_str)
    }
}

// Custom display implementation to print the block hash in little endian
impl fmt::Display for L1BlockId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut bytes = self.0.0;
        bytes.reverse();
        let mut buf = [0u8; 64]; // 32 bytes * 2 for hex
        encode_to_slice(bytes, &mut buf).expect("buf: enc hex");
        // SAFETY: hex encoding always produces valid UTF-8
        let hex_str = unsafe { str::from_utf8_unchecked(&buf) };
        f.write_str(hex_str)
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use ssz::{Decode, Encode};
    use strata_test_utils_ssz::ssz_proptest;

    use super::*;

    mod l1_block_id {
        use super::*;

        ssz_proptest!(
            L1BlockId,
            any::<[u8; 32]>().prop_map(Buf32::from),
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

        ssz_proptest!(
            L1BlockCommitment,
            (any::<u32>(), any::<[u8; 32]>()).prop_map(|(height, blkid)| {
                L1BlockCommitment::new(height, L1BlockId::from(Buf32::from(blkid)))
            })
        );

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
