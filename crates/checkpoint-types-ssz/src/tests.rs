//! Unit tests for SSZ checkpoint types.

use ssz::{Decode, Encode};
use ssz_types::FixedVector;
use tree_hash::TreeHash;

use crate::*;

// ========================================================================
// ChainstateRootTransition Tests
// ========================================================================

#[test]
fn test_chainstate_root_transition_ssz_roundtrip() {
    let original = ChainstateRootTransition {
        pre_state_root: FixedVector::from(vec![1u8; 32]),
        post_state_root: FixedVector::from(vec![2u8; 32]),
    };

    let encoded = original.as_ssz_bytes();
    let decoded =
        ChainstateRootTransition::from_ssz_bytes(&encoded).expect("decode should succeed");

    assert_eq!(original.pre_state_root, decoded.pre_state_root);
    assert_eq!(original.post_state_root, decoded.post_state_root);
}

#[test]
fn test_chainstate_root_transition_tree_hash() {
    let transition = ChainstateRootTransition {
        pre_state_root: FixedVector::from(vec![0xAAu8; 32]),
        post_state_root: FixedVector::from(vec![0xBBu8; 32]),
    };

    let root = transition.tree_hash_root();
    assert_ne!(root.0, [0u8; 32]);
}

// ========================================================================
// BatchTransition Tests
// ========================================================================

#[test]
fn test_batch_transition_ssz_roundtrip() {
    let original = BatchTransition {
        epoch: 5,
        pre_state_root: FixedVector::from(vec![3u8; 32]),
        post_state_root: FixedVector::from(vec![4u8; 32]),
    };

    let encoded = original.as_ssz_bytes();
    let decoded = BatchTransition::from_ssz_bytes(&encoded).expect("decode should succeed");

    assert_eq!(original.epoch, decoded.epoch);
    assert_eq!(original.pre_state_root, decoded.pre_state_root);
    assert_eq!(original.post_state_root, decoded.post_state_root);
}

#[test]
fn test_batch_transition_tree_hash() {
    let transition = BatchTransition {
        epoch: 10,
        pre_state_root: FixedVector::from(vec![0xCCu8; 32]),
        post_state_root: FixedVector::from(vec![0xDDu8; 32]),
    };

    let root = transition.tree_hash_root();
    assert_ne!(root.0, [0u8; 32]);
}

// ========================================================================
// BatchInfo Tests
// ========================================================================

#[test]
fn test_batch_info_ssz_roundtrip() {
    let original = BatchInfo {
        epoch: 7,
        l1_range_start_height: 100,
        l1_range_start_blkid: FixedVector::from(vec![5u8; 32]),
        l1_range_end_height: 200,
        l1_range_end_blkid: FixedVector::from(vec![6u8; 32]),
        l2_range_start_slot: 1000,
        l2_range_start_blkid: FixedVector::from(vec![7u8; 32]),
        l2_range_end_slot: 2000,
        l2_range_end_blkid: FixedVector::from(vec![8u8; 32]),
    };

    let encoded = original.as_ssz_bytes();
    let decoded = BatchInfo::from_ssz_bytes(&encoded).expect("decode should succeed");

    assert_eq!(original.epoch, decoded.epoch);
    assert_eq!(
        original.l1_range_start_height,
        decoded.l1_range_start_height
    );
    assert_eq!(original.l2_range_end_slot, decoded.l2_range_end_slot);
}

#[test]
fn test_batch_info_tree_hash() {
    let batch_info = BatchInfo {
        epoch: 3,
        l1_range_start_height: 50,
        l1_range_start_blkid: FixedVector::from(vec![0xEEu8; 32]),
        l1_range_end_height: 150,
        l1_range_end_blkid: FixedVector::from(vec![0xFFu8; 32]),
        l2_range_start_slot: 500,
        l2_range_start_blkid: FixedVector::from(vec![0x11u8; 32]),
        l2_range_end_slot: 1500,
        l2_range_end_blkid: FixedVector::from(vec![0x22u8; 32]),
    };

    let root = batch_info.tree_hash_root();
    assert_ne!(root.0, [0u8; 32]);
}

// ========================================================================
// EpochSummary Tests
// ========================================================================

#[test]
fn test_epoch_summary_ssz_roundtrip() {
    let original = EpochSummary {
        epoch: 15,
        terminal_slot: 3000,
        terminal_blkid: FixedVector::from(vec![9u8; 32]),
        prev_terminal_slot: 2500,
        prev_terminal_blkid: FixedVector::from(vec![10u8; 32]),
        new_l1_height: 300,
        new_l1_blkid: FixedVector::from(vec![11u8; 32]),
        final_state: FixedVector::from(vec![12u8; 32]),
    };

    let encoded = original.as_ssz_bytes();
    let decoded = EpochSummary::from_ssz_bytes(&encoded).expect("decode should succeed");

    assert_eq!(original.epoch, decoded.epoch);
    assert_eq!(original.terminal_slot, decoded.terminal_slot);
    assert_eq!(original.new_l1_height, decoded.new_l1_height);
}

#[test]
fn test_epoch_summary_tree_hash() {
    let summary = EpochSummary {
        epoch: 20,
        terminal_slot: 4000,
        terminal_blkid: FixedVector::from(vec![0x33u8; 32]),
        prev_terminal_slot: 3500,
        prev_terminal_blkid: FixedVector::from(vec![0x44u8; 32]),
        new_l1_height: 400,
        new_l1_blkid: FixedVector::from(vec![0x55u8; 32]),
        final_state: FixedVector::from(vec![0x66u8; 32]),
    };

    let root = summary.tree_hash_root();
    assert_ne!(root.0, [0u8; 32]);
}

// ========================================================================
// CommitmentInfo Tests
// ========================================================================

#[test]
fn test_commitment_info_ssz_roundtrip() {
    let original = CommitmentInfo {
        blockhash: FixedVector::from(vec![13u8; 32]),
        txid: FixedVector::from(vec![14u8; 32]),
    };

    let encoded = original.as_ssz_bytes();
    let decoded = CommitmentInfo::from_ssz_bytes(&encoded).expect("decode should succeed");

    assert_eq!(original.blockhash, decoded.blockhash);
    assert_eq!(original.txid, decoded.txid);
}

#[test]
fn test_commitment_info_tree_hash() {
    let info = CommitmentInfo {
        blockhash: FixedVector::from(vec![0x77u8; 32]),
        txid: FixedVector::from(vec![0x88u8; 32]),
    };

    let root = info.tree_hash_root();
    assert_ne!(root.0, [0u8; 32]);
}

// ========================================================================
// Property Tests
// ========================================================================

#[test]
fn test_tree_hash_deterministic() {
    let batch1 = BatchInfo {
        epoch: 5,
        l1_range_start_height: 10,
        l1_range_start_blkid: FixedVector::from(vec![0x99u8; 32]),
        l1_range_end_height: 20,
        l1_range_end_blkid: FixedVector::from(vec![0xAAu8; 32]),
        l2_range_start_slot: 100,
        l2_range_start_blkid: FixedVector::from(vec![0xBBu8; 32]),
        l2_range_end_slot: 200,
        l2_range_end_blkid: FixedVector::from(vec![0xCCu8; 32]),
    };

    let batch2 = BatchInfo {
        epoch: 5,
        l1_range_start_height: 10,
        l1_range_start_blkid: FixedVector::from(vec![0x99u8; 32]),
        l1_range_end_height: 20,
        l1_range_end_blkid: FixedVector::from(vec![0xAAu8; 32]),
        l2_range_start_slot: 100,
        l2_range_start_blkid: FixedVector::from(vec![0xBBu8; 32]),
        l2_range_end_slot: 200,
        l2_range_end_blkid: FixedVector::from(vec![0xCCu8; 32]),
    };

    assert_eq!(batch1.tree_hash_root(), batch2.tree_hash_root());
}

#[test]
fn test_different_values_different_hashes() {
    let batch1 = BatchInfo {
        epoch: 5,
        l1_range_start_height: 10,
        l1_range_start_blkid: FixedVector::from(vec![0x99u8; 32]),
        l1_range_end_height: 20,
        l1_range_end_blkid: FixedVector::from(vec![0xAAu8; 32]),
        l2_range_start_slot: 100,
        l2_range_start_blkid: FixedVector::from(vec![0xBBu8; 32]),
        l2_range_end_slot: 200,
        l2_range_end_blkid: FixedVector::from(vec![0xCCu8; 32]),
    };

    let batch2 = BatchInfo {
        epoch: 6, // Different epoch
        l1_range_start_height: 10,
        l1_range_start_blkid: FixedVector::from(vec![0x99u8; 32]),
        l1_range_end_height: 20,
        l1_range_end_blkid: FixedVector::from(vec![0xAAu8; 32]),
        l2_range_start_slot: 100,
        l2_range_start_blkid: FixedVector::from(vec![0xBBu8; 32]),
        l2_range_end_slot: 200,
        l2_range_end_blkid: FixedVector::from(vec![0xCCu8; 32]),
    };

    assert_ne!(batch1.tree_hash_root(), batch2.tree_hash_root());
}

// ========================================================================
// Serialization Size Tests
// ========================================================================

#[test]
fn test_serialization_sizes() {
    // ChainstateRootTransition: 2 * Vector[byte, 32] = 64 bytes
    let transition = ChainstateRootTransition {
        pre_state_root: FixedVector::from(vec![0xFFu8; 32]),
        post_state_root: FixedVector::from(vec![0xFFu8; 32]),
    };
    assert_eq!(transition.as_ssz_bytes().len(), 64);

    // BatchTransition: uint64 + 2 * Vector[byte, 32] = 72 bytes
    let batch_transition = BatchTransition {
        epoch: 999,
        pre_state_root: FixedVector::from(vec![0xFFu8; 32]),
        post_state_root: FixedVector::from(vec![0xFFu8; 32]),
    };
    assert_eq!(batch_transition.as_ssz_bytes().len(), 72);

    // CommitmentInfo: 2 * Vector[byte, 32] = 64 bytes
    let info = CommitmentInfo {
        blockhash: FixedVector::from(vec![0xFFu8; 32]),
        txid: FixedVector::from(vec![0xFFu8; 32]),
    };
    assert_eq!(info.as_ssz_bytes().len(), 64);
}

// ========================================================================
// Batch Operations
// ========================================================================

#[test]
fn test_batch_serialization() {
    let transitions = [
        ChainstateRootTransition {
            pre_state_root: FixedVector::from(vec![1u8; 32]),
            post_state_root: FixedVector::from(vec![2u8; 32]),
        },
        ChainstateRootTransition {
            pre_state_root: FixedVector::from(vec![3u8; 32]),
            post_state_root: FixedVector::from(vec![4u8; 32]),
        },
        ChainstateRootTransition {
            pre_state_root: FixedVector::from(vec![5u8; 32]),
            post_state_root: FixedVector::from(vec![6u8; 32]),
        },
    ];

    // Serialize all
    let serialized: Vec<Vec<u8>> = transitions.iter().map(|t| t.as_ssz_bytes()).collect();

    // Deserialize all
    let deserialized: Vec<ChainstateRootTransition> = serialized
        .iter()
        .map(|bytes| {
            ChainstateRootTransition::from_ssz_bytes(bytes).expect("decode should succeed")
        })
        .collect();

    // Verify
    for (original, decoded) in transitions.iter().zip(deserialized.iter()) {
        assert_eq!(original.pre_state_root, decoded.pre_state_root);
        assert_eq!(original.post_state_root, decoded.post_state_root);
    }
}

#[test]
fn test_batch_merkleization() {
    let transitions = [
        BatchTransition {
            epoch: 1,
            pre_state_root: FixedVector::from(vec![0xAAu8; 32]),
            post_state_root: FixedVector::from(vec![0xBBu8; 32]),
        },
        BatchTransition {
            epoch: 2,
            pre_state_root: FixedVector::from(vec![0xCCu8; 32]),
            post_state_root: FixedVector::from(vec![0xDDu8; 32]),
        },
        BatchTransition {
            epoch: 3,
            pre_state_root: FixedVector::from(vec![0xEEu8; 32]),
            post_state_root: FixedVector::from(vec![0xFFu8; 32]),
        },
    ];

    // Compute all roots
    let roots: Vec<tree_hash::Hash256> = transitions.iter().map(|t| t.tree_hash_root()).collect();

    // Verify all roots are unique (no collisions)
    for i in 0..roots.len() {
        for j in (i + 1)..roots.len() {
            assert_ne!(roots[i], roots[j], "Tree hash collision detected");
        }
    }
}
