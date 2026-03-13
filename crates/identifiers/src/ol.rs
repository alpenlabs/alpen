use std::{fmt, str};

#[cfg(feature = "arbitrary")]
use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use ssz_derive::{Decode, Encode};
use strata_codec::Codec;

use crate::buf::Buf32;

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
    BorshSerialize,
    BorshDeserialize,
    Serialize,
    Deserialize,
    Encode,
    Decode,
)]
#[cfg_attr(feature = "arbitrary", derive(Arbitrary))]
pub struct OLBlockId(Buf32);

impl_buf_wrapper!(OLBlockId, Buf32, 32);
strata_codec::impl_wrapper_codec!(OLBlockId => Buf32);

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

/// Commitment to an OL block by ID at a particular slot.
#[derive(
    Copy,
    Clone,
    Debug,
    Eq,
    PartialEq,
    Hash,
    Default,
    Serialize,
    Deserialize,
    Encode,
    Decode,
    Codec,
    PartialOrd,
    Ord,
)]
#[cfg_attr(feature = "arbitrary", derive(Arbitrary))]
#[ssz(struct_behaviour = "container")]
pub struct OLBlockCommitment {
    pub slot: Slot,
    pub blkid: OLBlockId,
}

crate::impl_tree_hash_container!(OLBlockCommitment, [slot, blkid]);
crate::impl_ssz_type_info_fixed!(OLBlockCommitment, [Slot, OLBlockId]);
crate::impl_ssz_container_ref!(OLBlockCommitmentRef, OLBlockCommitment);

impl OLBlockCommitment {
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
        write!(f, "{}@{}", self.slot, self.blkid)
    }
}

// Use macro to generate Borsh implementations via SSZ (fixed-size, no length prefix)
crate::impl_borsh_via_ssz_fixed!(OLBlockCommitment);

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
    BorshSerialize,
    BorshDeserialize,
    Serialize,
    Deserialize,
    Encode,
    Decode,
)]
#[cfg_attr(feature = "arbitrary", derive(Arbitrary))]
pub struct OLTxId(Buf32);

impl_buf_wrapper!(OLTxId, Buf32, 32);
strata_codec::impl_wrapper_codec!(OLTxId => Buf32);

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
