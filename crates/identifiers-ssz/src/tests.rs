//! Unit tests for SSZ identifier types.

use ssz::{Decode, Encode};
use ssz_types::FixedVector;
use tree_hash::TreeHash;

use crate::*;

// ========================================================================
// L1BlockCommitment Tests
// ========================================================================

#[test]
fn test_l1_block_commitment_ssz_roundtrip() {
    let original = L1BlockCommitment {
        height: 12345,
        blkid: FixedVector::from(vec![42u8; 32]),
    };

    let encoded = original.as_ssz_bytes();
    let decoded = L1BlockCommitment::from_ssz_bytes(&encoded).expect("decode should succeed");

    assert_eq!(original.height, decoded.height);
    assert_eq!(original.blkid, decoded.blkid);
}

#[test]
fn test_l1_block_commitment_tree_hash() {
    let commitment = L1BlockCommitment {
        height: 999,
        blkid: FixedVector::from(vec![0xAAu8; 32]),
    };

    let root = commitment.tree_hash_root();
    // Tree hash root is a FixedBytes<32>, verify it's not all zeros
    assert_ne!(root.0, [0u8; 32]);
}

// ========================================================================
// L2BlockCommitment Tests
// ========================================================================

#[test]
fn test_l2_block_commitment_ssz_roundtrip() {
    let original = L2BlockCommitment {
        slot: 54321,
        blkid: FixedVector::from(vec![99u8; 32]),
    };

    let encoded = original.as_ssz_bytes();
    let decoded = L2BlockCommitment::from_ssz_bytes(&encoded).expect("decode should succeed");

    assert_eq!(original.slot, decoded.slot);
    assert_eq!(original.blkid, decoded.blkid);
}

#[test]
fn test_l2_block_commitment_tree_hash() {
    let commitment = L2BlockCommitment {
        slot: 777,
        blkid: FixedVector::from(vec![0xBBu8; 32]),
    };

    let root = commitment.tree_hash_root();
    assert_ne!(root.0, [0u8; 32]);
}

// ========================================================================
// EpochCommitment Tests
// ========================================================================

#[test]
fn test_epoch_commitment_ssz_roundtrip() {
    let original = EpochCommitment {
        epoch: 10,
        last_slot: 1000,
        last_blkid: FixedVector::from(vec![77u8; 32]),
    };

    let encoded = original.as_ssz_bytes();
    let decoded = EpochCommitment::from_ssz_bytes(&encoded).expect("decode should succeed");

    assert_eq!(original.epoch, decoded.epoch);
    assert_eq!(original.last_slot, decoded.last_slot);
    assert_eq!(original.last_blkid, decoded.last_blkid);
}

#[test]
fn test_epoch_commitment_tree_hash() {
    let commitment = EpochCommitment {
        epoch: 5,
        last_slot: 555,
        last_blkid: FixedVector::from(vec![0xCCu8; 32]),
    };

    let root = commitment.tree_hash_root();
    assert_ne!(root.0, [0u8; 32]);
}

// ========================================================================
// ExecBlockCommitment Tests
// ========================================================================

#[test]
fn test_exec_block_commitment_ssz_roundtrip() {
    let original = ExecBlockCommitment {
        slot: 88888,
        blkid: FixedVector::from(vec![55u8; 32]),
    };

    let encoded = original.as_ssz_bytes();
    let decoded = ExecBlockCommitment::from_ssz_bytes(&encoded).expect("decode should succeed");

    assert_eq!(original.slot, decoded.slot);
    assert_eq!(original.blkid, decoded.blkid);
}

#[test]
fn test_exec_block_commitment_tree_hash() {
    let commitment = ExecBlockCommitment {
        slot: 444,
        blkid: FixedVector::from(vec![0xDDu8; 32]),
    };

    let root = commitment.tree_hash_root();
    assert_ne!(root.0, [0u8; 32]);
}

// ========================================================================
// Property Tests
// ========================================================================

#[test]
fn test_tree_hash_deterministic() {
    let commitment1 = L1BlockCommitment {
        height: 123,
        blkid: FixedVector::from(vec![0xAAu8; 32]),
    };
    let commitment2 = L1BlockCommitment {
        height: 123,
        blkid: FixedVector::from(vec![0xAAu8; 32]),
    };

    assert_eq!(commitment1.tree_hash_root(), commitment2.tree_hash_root());
}

#[test]
fn test_different_values_different_hashes() {
    let commitment1 = L1BlockCommitment {
        height: 123,
        blkid: FixedVector::from(vec![0xAAu8; 32]),
    };
    let commitment2 = L1BlockCommitment {
        height: 124,
        blkid: FixedVector::from(vec![0xAAu8; 32]),
    };

    assert_ne!(commitment1.tree_hash_root(), commitment2.tree_hash_root());
}

// ========================================================================
// Batch Operations
// ========================================================================

#[test]
fn test_batch_serialization() {
    let commitments = [
        L1BlockCommitment {
            height: 1,
            blkid: FixedVector::from(vec![1u8; 32]),
        },
        L1BlockCommitment {
            height: 2,
            blkid: FixedVector::from(vec![2u8; 32]),
        },
        L1BlockCommitment {
            height: 3,
            blkid: FixedVector::from(vec![3u8; 32]),
        },
    ];

    // Serialize all
    let serialized: Vec<Vec<u8>> = commitments.iter().map(|c| c.as_ssz_bytes()).collect();

    // Deserialize all
    let deserialized: Vec<L1BlockCommitment> = serialized
        .iter()
        .map(|bytes| L1BlockCommitment::from_ssz_bytes(bytes).expect("decode should succeed"))
        .collect();

    // Verify
    for (original, decoded) in commitments.iter().zip(deserialized.iter()) {
        assert_eq!(original.height, decoded.height);
        assert_eq!(original.blkid, decoded.blkid);
    }
}

#[test]
fn test_batch_merkleization() {
    let commitments = [
        L2BlockCommitment {
            slot: 100,
            blkid: FixedVector::from(vec![0xAAu8; 32]),
        },
        L2BlockCommitment {
            slot: 200,
            blkid: FixedVector::from(vec![0xBBu8; 32]),
        },
        L2BlockCommitment {
            slot: 300,
            blkid: FixedVector::from(vec![0xCCu8; 32]),
        },
    ];

    // Compute all roots
    let roots: Vec<tree_hash::Hash256> = commitments.iter().map(|c| c.tree_hash_root()).collect();

    // Verify all roots are unique (no collisions)
    for i in 0..roots.len() {
        for j in (i + 1)..roots.len() {
            assert_ne!(roots[i], roots[j], "Tree hash collision detected");
        }
    }
}

// ========================================================================
// Serialization Size Tests
// ========================================================================

#[test]
fn test_serialization_size_consistency() {
    // L1BlockCommitment: uint64 (8 bytes) + Vector[byte, 32] (32 bytes) = 40 bytes
    let l1 = L1BlockCommitment {
        height: 999,
        blkid: FixedVector::from(vec![0xFFu8; 32]),
    };
    assert_eq!(l1.as_ssz_bytes().len(), 40);

    // L2BlockCommitment: uint64 (8 bytes) + Vector[byte, 32] (32 bytes) = 40 bytes
    let l2 = L2BlockCommitment {
        slot: 999,
        blkid: FixedVector::from(vec![0xFFu8; 32]),
    };
    assert_eq!(l2.as_ssz_bytes().len(), 40);

    // EpochCommitment: uint64 + uint64 + Vector[byte, 32] = 48 bytes
    let epoch = EpochCommitment {
        epoch: 10,
        last_slot: 999,
        last_blkid: FixedVector::from(vec![0xFFu8; 32]),
    };
    assert_eq!(epoch.as_ssz_bytes().len(), 48);

    // ExecBlockCommitment: uint64 + Vector[byte, 32] = 40 bytes
    let exec = ExecBlockCommitment {
        slot: 999,
        blkid: FixedVector::from(vec![0xFFu8; 32]),
    };
    assert_eq!(exec.as_ssz_bytes().len(), 40);
}

#[test]
fn test_mixed_type_merkleization() {
    let l1 = L1BlockCommitment {
        height: 100,
        blkid: FixedVector::from(vec![0xAAu8; 32]),
    };

    let l2 = L2BlockCommitment {
        slot: 100,
        blkid: FixedVector::from(vec![0xAAu8; 32]),
    };

    let epoch = EpochCommitment {
        epoch: 100,
        last_slot: 100,
        last_blkid: FixedVector::from(vec![0xAAu8; 32]),
    };

    // L1 and L2 have identical SSZ structure (uint64 + Vector[byte, 32])
    // so they produce the same merkle root for the same field values.
    // This is expected SSZ behavior - the hash depends only on structure and values.
    let l1_root = l1.tree_hash_root();
    let l2_root = l2.tree_hash_root();
    assert_eq!(l1_root, l2_root);

    // EpochCommitment has a different structure (uint64 + uint64 + Vector[byte, 32])
    // so it produces a different root even with similar values
    let epoch_root = epoch.tree_hash_root();
    assert_ne!(l1_root, epoch_root);
}
