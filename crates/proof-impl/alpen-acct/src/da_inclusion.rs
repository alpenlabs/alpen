//! L1 DA inclusion helpers.

use strata_crypto::hash::sha256d;
use strata_ee_acct_runtime::{ArchivedBitcoinMerkleProof, BitcoinMerkleProof};

fn bitcoin_hash_pair(left: [u8; 32], right: [u8; 32]) -> [u8; 32] {
    let mut pair = [0u8; 64];
    pair[..32].copy_from_slice(&left);
    pair[32..].copy_from_slice(&right);

    *sha256d(&pair).as_ref()
}

/// Computes a Bitcoin-style Merkle root from a leaf and inclusion path.
///
/// `siblings` is ordered leaf-first. `position` is the leaf index in the
/// bottom layer; bit `i` selects whether `siblings[i]` is on the left or right
/// of the running hash at level `i`.
pub fn bitcoin_merkle_root(
    leaf_hash: [u8; 32],
    siblings: &[[u8; 32]],
    mut position: u32,
) -> [u8; 32] {
    let mut root = leaf_hash;

    for sibling in siblings {
        root = if position & 1 == 0 {
            bitcoin_hash_pair(root, *sibling)
        } else {
            bitcoin_hash_pair(*sibling, root)
        };
        position >>= 1;
    }

    root
}

/// Computes the Merkle root described by a [`BitcoinMerkleProof`].
pub fn bitcoin_merkle_root_from_proof(leaf_hash: [u8; 32], proof: &BitcoinMerkleProof) -> [u8; 32] {
    bitcoin_merkle_root(leaf_hash, proof.siblings(), proof.position())
}

/// Computes the Merkle root described by an archived [`BitcoinMerkleProof`].
pub fn bitcoin_merkle_root_from_archived_proof(
    leaf_hash: [u8; 32],
    proof: &ArchivedBitcoinMerkleProof,
) -> [u8; 32] {
    bitcoin_merkle_root(leaf_hash, proof.siblings(), proof.position())
}

#[cfg(test)]
mod tests {
    use rkyv::rancor::Error as RkyvError;
    use strata_ee_acct_runtime::{ArchivedBitcoinMerkleProof, BitcoinMerkleProof};

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
    fn bitcoin_merkle_root_with_empty_path_is_leaf() {
        let leaf = [0xAA; 32];

        assert_eq!(bitcoin_merkle_root(leaf, &[], 0), leaf);
    }

    #[test]
    fn bitcoin_merkle_root_respects_position_bits() {
        let left = [0x00; 32];
        let right = [0x11; 32];
        let expected = hex32("127e4900feebf53bb61ecc03d9a628da770e4f8ef65cfd6d40852cd9a553b3d5");

        assert_eq!(bitcoin_merkle_root(left, &[right], 0), expected);
        assert_eq!(bitcoin_merkle_root(right, &[left], 1), expected);
    }

    #[test]
    fn bitcoin_merkle_root_handles_multi_level_paths() {
        let leaf = [0x22; 32];
        let first_sibling = [0x22; 32];
        let left_subtree =
            hex32("127e4900feebf53bb61ecc03d9a628da770e4f8ef65cfd6d40852cd9a553b3d5");
        let expected = hex32("16c94d996b5f836f454a3e1f9be1df8ba65d708d54b8ae84f52122b9c856e928");

        assert_eq!(
            bitcoin_merkle_root(leaf, &[first_sibling, left_subtree], 2),
            expected
        );
    }

    #[test]
    fn bitcoin_merkle_root_from_proof_uses_proof_position() {
        let proof = BitcoinMerkleProof::new(vec![[0x11; 32]], 0);
        let expected = hex32("127e4900feebf53bb61ecc03d9a628da770e4f8ef65cfd6d40852cd9a553b3d5");

        assert_eq!(bitcoin_merkle_root_from_proof([0x00; 32], &proof), expected);
    }

    #[test]
    fn bitcoin_merkle_root_from_archived_proof_uses_archived_position() {
        let proof = BitcoinMerkleProof::new(vec![[0x11; 32]], 0);
        let bytes = rkyv::to_bytes::<RkyvError>(&proof).unwrap();
        let archived = rkyv::access::<ArchivedBitcoinMerkleProof, RkyvError>(&bytes).unwrap();
        let expected = hex32("127e4900feebf53bb61ecc03d9a628da770e4f8ef65cfd6d40852cd9a553b3d5");

        assert_eq!(
            bitcoin_merkle_root_from_archived_proof([0x00; 32], archived),
            expected
        );
    }
}
