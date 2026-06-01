//! Bitcoin merkle-tree primitives shared between witness construction (host)
//! and inclusion verification (guest).
//!
//! Bitcoin uses double-SHA-256 for wtxids and merkle-pair hashing; see BIP-141
//! for the wtxid / witness-root commitment these helpers target:
//! <https://github.com/bitcoin/bips/blob/master/bip-0141.mediawiki#commitment-structure>.

use bitcoin::{hashes::Hash as _, Transaction};
use strata_crypto::hash::sha256d;

/// Hashes one Bitcoin merkle-tree level as `SHA256(SHA256(left || right))`.
pub fn hash_pair_sha256d(left: [u8; 32], right: [u8; 32]) -> [u8; 32] {
    let mut pair = [0u8; 64];
    pair[..32].copy_from_slice(&left);
    pair[32..].copy_from_slice(&right);
    sha256d(&pair).0
}

/// Recomputes a Bitcoin-style Merkle root from a leaf and its inclusion path.
///
/// `siblings` is ordered leaf-first. `position` is the leaf index in the
/// bottom layer; bit `i` selects whether `siblings[i]` is on the left or right
/// of the running hash at level `i`. Comparing the result against a known root
/// is how an inclusion proof is verified.
pub fn compute_bitcoin_merkle_root_from_proof(
    leaf_hash: [u8; 32],
    siblings: &[[u8; 32]],
    mut position: u32,
) -> [u8; 32] {
    let mut root = leaf_hash;

    for sibling in siblings {
        root = if position & 1 == 0 {
            hash_pair_sha256d(root, *sibling)
        } else {
            hash_pair_sha256d(*sibling, root)
        };
        position >>= 1;
    }

    root
}

/// Computes the Bitcoin merkle root over a list of leaves, duplicating the
/// trailing leaf at any level with an odd node count (Bitcoin's construction
/// rule).
///
/// # Panics
///
/// Panics if `leaves` is empty.
pub fn compute_bitcoin_merkle_root_from_leaves(leaves: &[[u8; 32]]) -> [u8; 32] {
    assert!(!leaves.is_empty(), "merkle root requires at least one leaf");
    let mut cur_level = leaves.to_vec();
    while cur_level.len() > 1 {
        if cur_level.len() % 2 == 1 {
            cur_level.push(*cur_level.last().expect("non-empty level"));
        }
        cur_level = cur_level
            .chunks(2)
            .map(|pair| hash_pair_sha256d(pair[0], pair[1]))
            .collect();
    }
    cur_level[0]
}

/// Builds the leaf-first sibling path proving inclusion of `leaves[idx]` in
/// the Bitcoin merkle root of `leaves` (using the same odd-duplication rule
/// as [`compute_bitcoin_merkle_root_from_leaves`]).
///
/// # Panics
///
/// Panics if `idx >= leaves.len()`.
pub fn compute_bitcoin_inclusion_proof(leaves: &[[u8; 32]], idx: u32) -> Vec<[u8; 32]> {
    assert!(
        (idx as usize) < leaves.len(),
        "idx {idx} out of bounds for {} leaves",
        leaves.len()
    );

    let mut cur_level = leaves.to_vec();
    let mut cur_idx = idx;
    let depth = (usize::BITS - cur_level.len().leading_zeros()) as usize;
    let mut siblings = Vec::with_capacity(depth);

    while cur_level.len() > 1 {
        if cur_level.len() % 2 == 1 {
            cur_level.push(*cur_level.last().expect("non-empty level"));
        }

        siblings.push(cur_level[(cur_idx ^ 1) as usize]);

        cur_level = cur_level
            .chunks(2)
            .map(|pair| hash_pair_sha256d(pair[0], pair[1]))
            .collect();
        cur_idx >>= 1;
    }

    siblings
}

/// Returns the BIP-141 wtxid leaves for `txs`, zeroing the coinbase leaf
/// (leaf 0) per the witness-commitment rule.
pub fn wtxid_leaves(txs: &[Transaction]) -> Vec<[u8; 32]> {
    txs.iter()
        .enumerate()
        .map(|(i, tx)| {
            if i == 0 {
                [0u8; 32]
            } else {
                tx.compute_wtxid().to_byte_array()
            }
        })
        .collect()
}

/// Computes the BIP-141 witness transaction Merkle root for a block's tx list.
///
/// # Panics
///
/// Panics if `txs` is empty; the list must contain at least the coinbase tx.
pub fn wtxids_root_from_txs(txs: &[Transaction]) -> [u8; 32] {
    assert!(
        !txs.is_empty(),
        "wtxids root requires at least the coinbase tx"
    );
    compute_bitcoin_merkle_root_from_leaves(&wtxid_leaves(txs))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex32(s: &str) -> [u8; 32] {
        assert_eq!(s.len(), 64);
        let mut out = [0u8; 32];
        for i in 0..32 {
            out[i] = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).unwrap();
        }
        out
    }

    #[test]
    fn merkle_root_from_proof_with_empty_path_is_leaf() {
        let leaf = [0xAA; 32];
        assert_eq!(compute_bitcoin_merkle_root_from_proof(leaf, &[], 0), leaf);
    }

    #[test]
    fn merkle_root_from_proof_respects_position_bits() {
        let left = [0x00; 32];
        let right = [0x11; 32];
        let expected = hex32("127e4900feebf53bb61ecc03d9a628da770e4f8ef65cfd6d40852cd9a553b3d5");

        assert_eq!(
            compute_bitcoin_merkle_root_from_proof(left, &[right], 0),
            expected
        );
        assert_eq!(
            compute_bitcoin_merkle_root_from_proof(right, &[left], 1),
            expected
        );
    }

    #[test]
    fn inclusion_proof_matches_built_root() {
        let leaves: Vec<[u8; 32]> = (0u8..5).map(|i| [i; 32]).collect();
        let root = compute_bitcoin_merkle_root_from_leaves(&leaves);

        for (idx, leaf) in leaves.iter().enumerate() {
            let siblings = compute_bitcoin_inclusion_proof(&leaves, idx as u32);
            assert_eq!(
                compute_bitcoin_merkle_root_from_proof(*leaf, &siblings, idx as u32),
                root,
                "idx={idx}"
            );
        }
    }
}
