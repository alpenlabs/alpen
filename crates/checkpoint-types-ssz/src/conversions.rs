//! Conversions between Borsh-based checkpoint types and SSZ types.

use ssz_types::FixedVector;
use strata_checkpoint_types as borsh;
use strata_identifiers as ids;

// Helper to convert FixedVector<u8, 32> to [u8; 32]
fn fixed_vector_to_array(fv: FixedVector<u8, 32>) -> [u8; 32] {
    let vec: Vec<u8> = fv.into();
    vec.try_into()
        .expect("FixedVector<u8, 32> should convert to [u8; 32]")
}

// ============================================================================
// ChainstateRootTransition
// ============================================================================

impl From<borsh::ChainstateRootTransition> for crate::ChainstateRootTransition {
    fn from(value: borsh::ChainstateRootTransition) -> Self {
        Self {
            pre_state_root: FixedVector::from(value.pre_state_root.as_ref().to_vec()),
            post_state_root: FixedVector::from(value.post_state_root.as_ref().to_vec()),
        }
    }
}

impl From<crate::ChainstateRootTransition> for borsh::ChainstateRootTransition {
    fn from(value: crate::ChainstateRootTransition) -> Self {
        Self {
            pre_state_root: ids::Buf32::from(fixed_vector_to_array(value.pre_state_root)),
            post_state_root: ids::Buf32::from(fixed_vector_to_array(value.post_state_root)),
        }
    }
}

// ============================================================================
// BatchTransition
// ============================================================================

impl From<borsh::BatchTransition> for crate::BatchTransition {
    fn from(value: borsh::BatchTransition) -> Self {
        Self {
            epoch: value.epoch,
            pre_state_root: FixedVector::from(
                value.chainstate_transition.pre_state_root.as_ref().to_vec(),
            ),
            post_state_root: FixedVector::from(
                value
                    .chainstate_transition
                    .post_state_root
                    .as_ref()
                    .to_vec(),
            ),
        }
    }
}

impl From<crate::BatchTransition> for borsh::BatchTransition {
    fn from(value: crate::BatchTransition) -> Self {
        Self {
            epoch: value.epoch,
            chainstate_transition: borsh::ChainstateRootTransition {
                pre_state_root: ids::Buf32::from(fixed_vector_to_array(value.pre_state_root)),
                post_state_root: ids::Buf32::from(fixed_vector_to_array(value.post_state_root)),
            },
        }
    }
}

// ============================================================================
// BatchInfo
// ============================================================================

impl From<borsh::BatchInfo> for crate::BatchInfo {
    fn from(value: borsh::BatchInfo) -> Self {
        let (l1_start, l1_end) = value.l1_range;
        let (l2_start, l2_end) = value.l2_range;

        Self {
            epoch: value.epoch,
            l1_range_start_height: l1_start.height_u64(),
            l1_range_start_blkid: FixedVector::from(l1_start.blkid().as_ref().to_vec()),
            l1_range_end_height: l1_end.height_u64(),
            l1_range_end_blkid: FixedVector::from(l1_end.blkid().as_ref().to_vec()),
            l2_range_start_slot: l2_start.slot(),
            l2_range_start_blkid: FixedVector::from(l2_start.blkid().as_ref().to_vec()),
            l2_range_end_slot: l2_end.slot(),
            l2_range_end_blkid: FixedVector::from(l2_end.blkid().as_ref().to_vec()),
        }
    }
}

impl From<crate::BatchInfo> for borsh::BatchInfo {
    fn from(value: crate::BatchInfo) -> Self {
        let l1_start = ids::L1BlockCommitment::from_height_u64(
            value.l1_range_start_height,
            ids::L1BlockId::from(ids::Buf32::from(fixed_vector_to_array(
                value.l1_range_start_blkid,
            ))),
        )
        .expect("valid L1 height");

        let l1_end = ids::L1BlockCommitment::from_height_u64(
            value.l1_range_end_height,
            ids::L1BlockId::from(ids::Buf32::from(fixed_vector_to_array(
                value.l1_range_end_blkid,
            ))),
        )
        .expect("valid L1 height");

        let l2_start = ids::L2BlockCommitment::new(
            value.l2_range_start_slot,
            ids::L2BlockId::from(ids::Buf32::from(fixed_vector_to_array(
                value.l2_range_start_blkid,
            ))),
        );

        let l2_end = ids::L2BlockCommitment::new(
            value.l2_range_end_slot,
            ids::L2BlockId::from(ids::Buf32::from(fixed_vector_to_array(
                value.l2_range_end_blkid,
            ))),
        );

        Self {
            epoch: value.epoch,
            l1_range: (l1_start, l1_end),
            l2_range: (l2_start, l2_end),
        }
    }
}

// ============================================================================
// EpochSummary
// ============================================================================

impl From<borsh::EpochSummary> for crate::EpochSummary {
    fn from(value: borsh::EpochSummary) -> Self {
        Self {
            epoch: value.epoch(),
            terminal_slot: value.terminal().slot(),
            terminal_blkid: FixedVector::from(value.terminal().blkid().as_ref().to_vec()),
            prev_terminal_slot: value.prev_terminal().slot(),
            prev_terminal_blkid: FixedVector::from(value.prev_terminal().blkid().as_ref().to_vec()),
            new_l1_height: value.new_l1().height_u64(),
            new_l1_blkid: FixedVector::from(value.new_l1().blkid().as_ref().to_vec()),
            final_state: FixedVector::from(value.final_state().as_ref().to_vec()),
        }
    }
}

impl From<crate::EpochSummary> for borsh::EpochSummary {
    fn from(value: crate::EpochSummary) -> Self {
        let terminal = ids::L2BlockCommitment::new(
            value.terminal_slot,
            ids::L2BlockId::from(ids::Buf32::from(fixed_vector_to_array(
                value.terminal_blkid,
            ))),
        );

        let prev_terminal = ids::L2BlockCommitment::new(
            value.prev_terminal_slot,
            ids::L2BlockId::from(ids::Buf32::from(fixed_vector_to_array(
                value.prev_terminal_blkid,
            ))),
        );

        let new_l1 = ids::L1BlockCommitment::from_height_u64(
            value.new_l1_height,
            ids::L1BlockId::from(ids::Buf32::from(fixed_vector_to_array(value.new_l1_blkid))),
        )
        .expect("valid L1 height");

        let final_state = ids::Buf32::from(fixed_vector_to_array(value.final_state));

        Self::new(value.epoch, terminal, prev_terminal, new_l1, final_state)
    }
}

// ============================================================================
// CommitmentInfo
// ============================================================================

impl From<borsh::CommitmentInfo> for crate::CommitmentInfo {
    fn from(value: borsh::CommitmentInfo) -> Self {
        Self {
            blockhash: FixedVector::from(value.blockhash.as_ref().to_vec()),
            txid: FixedVector::from(value.txid.as_ref().to_vec()),
        }
    }
}

impl From<crate::CommitmentInfo> for borsh::CommitmentInfo {
    fn from(value: crate::CommitmentInfo) -> Self {
        Self {
            blockhash: ids::Buf32::from(fixed_vector_to_array(value.blockhash)),
            txid: ids::Buf32::from(fixed_vector_to_array(value.txid)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chainstate_root_transition_conversion() {
        let borsh_val = borsh::ChainstateRootTransition {
            pre_state_root: ids::Buf32::from([1u8; 32]),
            post_state_root: ids::Buf32::from([2u8; 32]),
        };

        let ssz_val: crate::ChainstateRootTransition = borsh_val.into();
        let borsh_roundtrip: borsh::ChainstateRootTransition = ssz_val.into();

        assert_eq!(borsh_roundtrip.pre_state_root.as_ref(), &[1u8; 32]);
        assert_eq!(borsh_roundtrip.post_state_root.as_ref(), &[2u8; 32]);
    }

    #[test]
    fn test_batch_transition_conversion() {
        let borsh_val = borsh::BatchTransition {
            epoch: 5,
            chainstate_transition: borsh::ChainstateRootTransition {
                pre_state_root: ids::Buf32::from([3u8; 32]),
                post_state_root: ids::Buf32::from([4u8; 32]),
            },
        };

        let ssz_val: crate::BatchTransition = borsh_val.into();
        assert_eq!(ssz_val.epoch, 5);

        let borsh_roundtrip: borsh::BatchTransition = ssz_val.into();
        assert_eq!(borsh_roundtrip.epoch, 5);
        assert_eq!(
            borsh_roundtrip
                .chainstate_transition
                .pre_state_root
                .as_ref(),
            &[3u8; 32]
        );
    }

    #[test]
    fn test_batch_info_conversion() {
        let l1_start = ids::L1BlockCommitment::from_height_u64(
            100,
            ids::L1BlockId::from(ids::Buf32::from([5u8; 32])),
        )
        .unwrap();
        let l1_end = ids::L1BlockCommitment::from_height_u64(
            200,
            ids::L1BlockId::from(ids::Buf32::from([6u8; 32])),
        )
        .unwrap();
        let l2_start =
            ids::L2BlockCommitment::new(1000, ids::L2BlockId::from(ids::Buf32::from([7u8; 32])));
        let l2_end =
            ids::L2BlockCommitment::new(2000, ids::L2BlockId::from(ids::Buf32::from([8u8; 32])));

        let borsh_val = borsh::BatchInfo::new(10, (l1_start, l1_end), (l2_start, l2_end));

        let ssz_val: crate::BatchInfo = borsh_val.into();
        assert_eq!(ssz_val.epoch, 10);
        assert_eq!(ssz_val.l1_range_start_height, 100);
        assert_eq!(ssz_val.l2_range_end_slot, 2000);

        let borsh_roundtrip: borsh::BatchInfo = ssz_val.into();
        assert_eq!(borsh_roundtrip.epoch, 10);
    }

    #[test]
    fn test_commitment_info_conversion() {
        let borsh_val = borsh::CommitmentInfo {
            blockhash: ids::Buf32::from([9u8; 32]),
            txid: ids::Buf32::from([10u8; 32]),
        };

        let ssz_val: crate::CommitmentInfo = borsh_val.into();
        let borsh_roundtrip: borsh::CommitmentInfo = ssz_val.into();

        assert_eq!(borsh_roundtrip.blockhash.as_ref(), &[9u8; 32]);
        assert_eq!(borsh_roundtrip.txid.as_ref(), &[10u8; 32]);
    }
}
