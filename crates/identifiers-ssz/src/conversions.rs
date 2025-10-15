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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_l1_block_commitment_borsh_conversion() {
        let borsh_commitment = borsh::L1BlockCommitment::from_height_u64(
            100,
            borsh::L1BlockId::from(borsh::Buf32::from([1u8; 32])),
        )
        .expect("valid height");

        let ssz_commitment: crate::L1BlockCommitment = borsh_commitment.into();
        assert_eq!(ssz_commitment.height, 100);

        let borsh_roundtrip: borsh::L1BlockCommitment = ssz_commitment.into();
        assert_eq!(borsh_roundtrip.height_u64(), 100);
    }

    #[test]
    fn test_l2_block_commitment_borsh_conversion() {
        let borsh_commitment = borsh::L2BlockCommitment::new(
            200,
            borsh::L2BlockId::from(borsh::Buf32::from([2u8; 32])),
        );

        let ssz_commitment: crate::L2BlockCommitment = borsh_commitment.into();
        assert_eq!(ssz_commitment.slot, 200);

        let borsh_roundtrip: borsh::L2BlockCommitment = ssz_commitment.into();
        assert_eq!(borsh_roundtrip.slot(), 200);
    }

    #[test]
    fn test_epoch_commitment_borsh_conversion() {
        let borsh_commitment = borsh::EpochCommitment::new(
            15,
            300,
            borsh::L2BlockId::from(borsh::Buf32::from([3u8; 32])),
        );

        let ssz_commitment: crate::EpochCommitment = borsh_commitment.into();
        assert_eq!(ssz_commitment.epoch, 15);
        assert_eq!(ssz_commitment.last_slot, 300);

        let borsh_roundtrip: borsh::EpochCommitment = ssz_commitment.into();
        assert_eq!(borsh_roundtrip.epoch(), 15);
        assert_eq!(borsh_roundtrip.last_slot(), 300);
    }

    #[test]
    fn test_exec_block_commitment_borsh_conversion() {
        let borsh_commitment = borsh::ExecBlockCommitment::new(400, borsh::Buf32::from([4u8; 32]));

        let ssz_commitment: crate::ExecBlockCommitment = borsh_commitment.into();
        assert_eq!(ssz_commitment.slot, 400);

        let borsh_roundtrip: borsh::ExecBlockCommitment = ssz_commitment.into();
        assert_eq!(borsh_roundtrip.slot(), 400);
    }
}
