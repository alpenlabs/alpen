use std::{cmp, fmt, str};

use arbitrary::Arbitrary;
use const_hex as hex;
use serde::{Deserialize, Serialize};
use ssz_derive::{Decode, Encode};
use strata_codec::{Codec, CodecError, Decoder, Encoder};

use crate::{buf::Buf32, ssz_generated::ssz::commitments::OLBlockCommitment};

pub type Slot = u64;
pub type Epoch = u32;

/// ID of an OL (Orchestration Layer) block, usually the hash of its root header.
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
    Serialize,
    Deserialize,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
    Encode,
    Decode,
)]
pub struct OLBlockId(Buf32);

impl_buf_wrapper!(OLBlockId, Buf32, 32);

// Manual TreeHash implementation for transparent wrapper
impl_ssz_transparent_buf32_wrapper!(OLBlockId);

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

impl crate::OLBlockCommitment {
    pub fn new(slot: Slot, blkid: OLBlockId) -> Self {
        Self { slot, blkid }
    }

    pub fn null() -> Self {
        Self::new(0, OLBlockId::from(Buf32::zero()))
    }

    pub fn slot(&self) -> Slot {
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
            str::from_utf8(&first_hex).expect("Failed to convert first hex bytes to UTF-8 string"),
            str::from_utf8(&last_hex).expect("Failed to convert last hex bytes to UTF-8 string")
        )
    }
}

impl Codec for OLBlockCommitment {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        self.slot.encode(enc)?;
        self.blkid.encode(enc)?;
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let slot = u64::decode(dec)?;
        let blkid = OLBlockId::decode(dec)?;
        Ok(Self { slot, blkid })
    }
}

impl<'a> arbitrary::Arbitrary<'a> for OLBlockCommitment {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        Ok(Self {
            slot: u.arbitrary()?,
            blkid: u.arbitrary()?,
        })
    }
}

impl Ord for OLBlockCommitment {
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        (self.slot, &self.blkid).cmp(&(other.slot, &other.blkid))
    }
}

impl PartialOrd for OLBlockCommitment {
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Alias for backward compatibility
pub type L2BlockCommitment = OLBlockCommitment;

/// ID of an OL (Orchestration Layer) transaction.
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
    Serialize,
    Deserialize,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
    Encode,
    Decode,
)]
pub struct OLTxId(Buf32);

impl_buf_wrapper!(OLTxId, Buf32, 32);

crate::impl_ssz_transparent_buf32_wrapper_copy!(OLTxId);

#[cfg(test)]
mod tests {
    use ssz::{Decode, Encode};
    use strata_test_utils_ssz::ssz_proptest;

    use super::*;
    use crate::test_utils::{buf32_strategy, ol_block_commitment_strategy};

    mod ol_block_id {
        use super::*;

        ssz_proptest!(
            OLBlockId,
            buf32_strategy(),
            transparent_wrapper_of(Buf32, from)
        );

        #[test]
        fn test_zero_ssz() {
            let zero = OLBlockId::from(Buf32::zero());
            let encoded = zero.as_ssz_bytes();
            let decoded = OLBlockId::from_ssz_bytes(&encoded).unwrap();
            assert_eq!(zero, decoded);
        }
    }

    mod ol_block_commitment {
        use super::*;

        ssz_proptest!(OLBlockCommitment, ol_block_commitment_strategy());

        #[test]
        fn test_zero_ssz() {
            let commitment = OLBlockCommitment::new(0, OLBlockId::from(Buf32::zero()));
            let encoded = commitment.as_ssz_bytes();
            let decoded = OLBlockCommitment::from_ssz_bytes(&encoded).unwrap();
            assert_eq!(commitment.slot(), decoded.slot());
            assert_eq!(commitment.blkid(), decoded.blkid());
        }
    }

    mod ol_tx_id {
        use super::*;

        ssz_proptest!(
            OLTxId,
            buf32_strategy(),
            transparent_wrapper_of(Buf32, from)
        );

        #[test]
        fn test_zero_ssz() {
            let zero = OLTxId::from(Buf32::zero());
            let encoded = zero.as_ssz_bytes();
            let decoded = OLTxId::from_ssz_bytes(&encoded).unwrap();
            assert_eq!(zero, decoded);
        }
    }
}
