//! Conversions between Borsh-based identifiers and SSZ types.
//!
//! This module provides `From` implementations for bidirectional conversion
//! between the legacy Borsh types in `strata-identifiers` and the new SSZ types.

use ssz_types::FixedVector;
use strata_identifiers as borsh;

// Helper to convert FixedVector<u8, 32> to [u8; 32]
fn fixed_vector_to_array(fv: FixedVector<u8, 32>) -> [u8; 32] {
    let vec: Vec<u8> = fv.into();
    vec.try_into()
        .expect("FixedVector<u8, 32> should convert to [u8; 32]")
}

// ============================================================================
// L1BlockCommitment
// ============================================================================

impl From<borsh::L1BlockCommitment> for crate::L1BlockCommitment {
    fn from(value: borsh::L1BlockCommitment) -> Self {
        Self {
            height: value.height_u64(),
            blkid: FixedVector::from(value.blkid().as_ref().to_vec()),
        }
    }
}

impl From<crate::L1BlockCommitment> for borsh::L1BlockCommitment {
    fn from(value: crate::L1BlockCommitment) -> Self {
        let blkid_bytes = fixed_vector_to_array(value.blkid);

        borsh::L1BlockCommitment::from_height_u64(
            value.height,
            borsh::L1BlockId::from(borsh::Buf32::from(blkid_bytes)),
        )
        .expect("height should be valid")
    }
}

// ============================================================================
// L2BlockCommitment (OLBlockCommitment)
// ============================================================================

impl From<borsh::L2BlockCommitment> for crate::L2BlockCommitment {
    fn from(value: borsh::L2BlockCommitment) -> Self {
        Self {
            slot: value.slot(),
            blkid: FixedVector::from(value.blkid().as_ref().to_vec()),
        }
    }
}

impl From<crate::L2BlockCommitment> for borsh::L2BlockCommitment {
    fn from(value: crate::L2BlockCommitment) -> Self {
        let blkid_bytes = fixed_vector_to_array(value.blkid);

        borsh::L2BlockCommitment::new(
            value.slot,
            borsh::L2BlockId::from(borsh::Buf32::from(blkid_bytes)),
        )
    }
}

// ============================================================================
// EpochCommitment
// ============================================================================

impl From<borsh::EpochCommitment> for crate::EpochCommitment {
    fn from(value: borsh::EpochCommitment) -> Self {
        Self {
            epoch: value.epoch(),
            last_slot: value.last_slot(),
            last_blkid: FixedVector::from(value.last_blkid().as_ref().to_vec()),
        }
    }
}

impl From<crate::EpochCommitment> for borsh::EpochCommitment {
    fn from(value: crate::EpochCommitment) -> Self {
        let blkid_bytes = fixed_vector_to_array(value.last_blkid);

        borsh::EpochCommitment::new(
            value.epoch,
            value.last_slot,
            borsh::L2BlockId::from(borsh::Buf32::from(blkid_bytes)),
        )
    }
}

// ============================================================================
// ExecBlockCommitment
// ============================================================================

impl From<borsh::ExecBlockCommitment> for crate::ExecBlockCommitment {
    fn from(value: borsh::ExecBlockCommitment) -> Self {
        Self {
            slot: value.slot(),
            blkid: FixedVector::from(value.blkid().as_ref().to_vec()),
        }
    }
}

impl From<crate::ExecBlockCommitment> for borsh::ExecBlockCommitment {
    fn from(value: crate::ExecBlockCommitment) -> Self {
        let blkid_bytes = fixed_vector_to_array(value.blkid);

        borsh::ExecBlockCommitment::new(value.slot, borsh::Buf32::from(blkid_bytes))
    }
}
